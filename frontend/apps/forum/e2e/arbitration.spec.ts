import { expect, test } from "@playwright/test";
import { member } from "./auth";
import {
  endEvidence,
  freshCode,
  grantRole,
  seedDispute,
  virtualAuthenticator,
} from "./staff-helpers";

// 仲裁（plan 第 7 节、ADR 0013）：争议订单与案件直接写库（本来由 indexer 从链上写入、worker 分配仲裁员）；
// 当事人在订单页提交陈述 → 举证期结束 → 管理员登记裁决 Passkey（邮箱验证码）→ 裁决用虚拟认证器签署 →
// 公示后当事人看到申诉入口，公示页不含任何身份。

test("arbitrator signs a ruling with the ruling passkey and the appeal window opens", async ({
  browser,
  baseURL,
}, info) => {
  test.skip(info.project.name !== "desktop", "虚拟认证器只跑桌面 Chromium");
  test.setTimeout(120_000);
  const base = baseURL ?? "";
  const buyer = await member(browser, base, "abuy", { username: true });
  const seller = await member(browser, base, "asel", { username: true });
  const arb = await member(browser, base, "arb", { username: true });
  try {
    await grantRole(arb.id, 3);
    const { orderId, caseId } = await seedDispute(buyer.id, seller.id, arb.id, 45);

    // —— 当事人：订单页的仲裁入口，举证期内提交陈述
    await buyer.page.goto(`/orders/${orderId}`);
    const panel = buyer.page.getByTestId("order-case");
    await expect(panel).toContainText(`仲裁案件 #${caseId}`);
    await expect(panel.getByRole("button", { name: "申诉" })).toHaveCount(0);
    await panel.getByRole("button", { name: "提交陈述 / 证据" }).click();
    await panel
      .getByLabel("陈述（只追加，提交后不能修改）")
      .fill("激活码提示已被使用，卖家拒绝更换");
    await panel.getByRole("button", { name: "提交陈述" }).click();
    await expect(panel.getByRole("button", { name: "提交陈述 / 证据" })).toBeVisible();
    await endEvidence(caseId);

    // —— 仲裁员：待办里登记裁决 Passkey（RP ID = 论坛主机）
    const credentials = await virtualAuthenticator(arb.page);
    await arb.page.goto("/arbitration");
    await expect(arb.page.getByTestId("queue-item")).toContainText(`案件 #${caseId}`);
    const setup = arb.page.getByTestId("ruling-passkey-setup");
    const code = await freshCode(arb.email, () =>
      setup.getByRole("button", { name: "发送验证码到我的邮箱" }).click(),
    );
    await setup.getByLabel("邮箱验证码").fill(code);
    await setup.getByRole("button", { name: "验证并创建" }).click();
    await expect(arb.page.getByTestId("ruling-passkey-status")).toContainText("已登记 1 个");
    expect(await credentials()).toBe(1);

    // —— 案件详情：证据、聊天哈希链、时间线，然后裁决
    await arb.page.goto(`/arbitration/${caseId}`);
    await expect(arb.page.getByTestId("case-statements")).toContainText("激活码提示已被使用");
    await expect(arb.page.getByTestId("case-chain-status")).toContainText("哈希链完整");
    await expect(arb.page.getByTestId("case-timeline")).toContainText("举证期结束");
    const form = arb.page.getByRole("form", { name: "裁决" });
    await form.getByLabel("争议本金判给买家（%）").fill("75");
    await expect(arb.page.getByTestId("ruling-estimate")).toContainText("33.75");
    await form
      .getByLabel("公开理由（对双方公开，写入审计日志）")
      .fill("聊天记录显示激活码发货前已被使用");
    await form.getByRole("button", { name: "确认仲裁员裁决" }).click();
    const sign = arb.page.getByRole("dialog", { name: "用裁决 Passkey 签署" });
    // 签署的金额来自服务端按链上订单的计算
    await expect(sign.getByTestId("ruling-intent")).toContainText("75%");
    await expect(sign.getByTestId("ruling-intent")).toContainText("11.25");
    await sign.getByRole("button", { name: "签署并提交" }).click();
    await expect(arb.page.getByRole("status")).toContainText("裁决已公示，进入 72 小时申诉期");
    await expect(arb.page.getByTestId("case-status")).toHaveText("已公示 · 申诉期");

    // —— 当事人：公示后看到裁决与申诉入口
    await seller.page.goto(`/orders/${orderId}`);
    const sellerPanel = seller.page.getByTestId("order-case");
    await expect(sellerPanel).toContainText("已公示 · 申诉期");
    await expect(sellerPanel).toContainText("本金 75% 判给买家");
    await expect(sellerPanel.getByRole("button", { name: "申诉" })).toBeVisible();

    // —— 公示页：任何人可看，不含当事人与仲裁员的身份
    const pub = await buyer.page.goto(`/arbitration/public/${caseId}`);
    expect(pub?.status()).toBe(200);
    await expect(buyer.page.getByTestId("ruling")).toContainText(
      "聊天记录显示激活码发货前已被使用",
    );
    const text = (await buyer.page.locator("main").textContent()) ?? "";
    for (const name of [buyer.username, seller.username, arb.username]) {
      expect(text).not.toContain(name ?? "");
    }

    // —— 申诉：每个案件一次，进入复核
    await sellerPanel.getByRole("button", { name: "申诉" }).click();
    const appeal = seller.page.getByRole("dialog", { name: "申诉" });
    await appeal.getByLabel("申诉理由").fill("激活码是买家激活后转卖的");
    await appeal.getByRole("button", { name: "提交申诉" }).click();
    await expect(sellerPanel).toContainText("申诉复核中");
    await expect(sellerPanel.getByRole("button", { name: "申诉" })).toHaveCount(0);

    // 不是当事人也不是本案仲裁员：案件不存在
    const outsider = await member(browser, base, "aout");
    try {
      expect((await outsider.page.goto(`/arbitration/${caseId}`))?.status()).toBe(404);
    } finally {
      await outsider.context.close();
    }
  } finally {
    await Promise.all([buyer.context.close(), seller.context.close(), arb.context.close()]);
  }
});
