import { expect, test } from "@playwright/test";
import { member } from "./auth";
import {
  ADMIN_URL,
  createTopic,
  forumId,
  freshCode,
  grantRole,
  virtualAuthenticator,
} from "./staff-helpers";

// 站长后台（ADR 0013）：站长用邮箱登录论坛 → 后台邮箱验证码绑定第一个后台 Passkey → 任命版主；
// 版主只能管自己的板块：界面上别的板块没有管理入口，绕过界面直接调接口也是 403，权限被收回后界面照常提示 403。
// 需要 forum-api 配 ADMIN_ORIGIN = ADMIN_URL、ADMIN_RP_ID = localhost，后台静态导出在 ADMIN_URL 上（见 e2e/README.md）。

test("owner enrolls an admin passkey and appoints a board moderator", async ({
  browser,
  baseURL,
}, info) => {
  test.skip(info.project.name !== "desktop", "后台与虚拟认证器只跑桌面 Chromium");
  test.setTimeout(120_000);
  const base = baseURL ?? "";
  const owner = await member(browser, base, "owner", { username: true });
  const mod = await member(browser, base, "mod", { username: true });
  const author = await member(browser, base, "auth", { username: true });
  try {
    await grantRole(owner.id, 1);
    // 后台在另一个源上：用同一浏览器上下文（论坛会话 Cookie）开新标签页，不带测试用的 CF-Connecting-IP
    // 请求头（跨源请求带上它会让 CORS 预检失败）
    const admin = await owner.context.newPage();
    // 假时钟：稍后快进 6 分钟，越过 5 分钟的再验证有效期
    await admin.clock.install();
    const credentials = await virtualAuthenticator(admin);
    const general = await forumId("general");
    const ownTopic = await createTopic(author.page, base, "general", "版主自己板块的主题");
    const secondTopic = await createTopic(author.page, base, "general", "稍后会被收回权限的主题");
    const otherTopic = await createTopic(author.page, base, "tech", "别的板块的主题");

    // —— 后台：没有后台 Passkey 时先走专用邮箱验证码，绑定即登录
    await admin.goto(ADMIN_URL);
    await admin.getByRole("button", { name: "用后台 Passkey 登录" }).click();
    await expect(admin.getByText("首次使用：绑定后台 Passkey")).toBeVisible();
    const code = await freshCode(owner.email, () =>
      admin.getByRole("button", { name: "发送验证码到站长邮箱" }).click(),
    );
    await admin.getByLabel("邮箱验证码").fill(code);
    await admin.getByRole("button", { name: "验证并创建后台 Passkey" }).click();
    await expect(admin.getByRole("heading", { name: "概览" })).toBeVisible();
    expect(await credentials()).toBe(1);
    await expect(
      admin.getByRole("table", { name: "后台 Passkey" }).locator("tbody tr"),
    ).toHaveCount(1);

    // 退出后用同一个 Passkey 登录（RP ID = 后台域名）
    await admin.getByRole("button", { name: "退出" }).click();
    await admin.getByRole("button", { name: "用后台 Passkey 登录" }).click();
    await expect(admin.getByRole("heading", { name: "概览" })).toBeVisible();

    // —— 任命版主（刚登录 = 刚验证过，5 分钟内的敏感操作不再弹再验证）
    await admin.goto(`${ADMIN_URL}/staff/`);
    const form = admin.getByRole("form", { name: "任命管理人员" });
    await form.getByLabel("用户").fill(mod.username ?? "");
    await admin.getByRole("option", { name: new RegExp(mod.username ?? "") }).click();
    await form.locator('select[name="role"]').selectOption("moderator");
    await form.locator('select[name="forumId"]').selectOption(String(general));
    await form.getByRole("button", { name: "任命" }).click();
    const row = admin.getByTestId("staff-row").filter({ hasText: mod.username ?? "" });
    await expect(row).toContainText("版主");
    await expect(row).toContainText("综合讨论");

    // —— 版主在自己的板块隐藏主题
    const m = mod.page;
    await m.goto(`/t/${ownTopic}`);
    await m.getByRole("button", { name: "管理", exact: true }).click();
    await m.getByRole("menuitem", { name: "隐藏主题" }).click();
    const dialog = m.getByRole("dialog", { name: "隐藏主题" });
    await dialog.locator('select[name="preset"]').selectOption("广告或引流");
    await dialog.getByRole("button", { name: "确认" }).click();
    await expect(dialog).toBeHidden();
    // 普通用户看不到隐藏的主题
    expect((await author.page.goto(`/t/${ownTopic}`))?.status()).toBe(404);

    // —— 别的板块：没有管理入口，直接调接口也被拒绝
    await m.goto(`/t/${otherTopic}`);
    await expect(m.getByRole("heading", { name: "别的板块的主题" })).toBeVisible();
    await expect(m.getByRole("button", { name: "管理", exact: true })).toHaveCount(0);
    const denied = await m.request.post(`/api/v1/topics/${otherTopic}/moderation`, {
      headers: { origin: new URL(base).origin },
      data: { action: "hide", reason: "违反版规" },
    });
    expect(denied.status()).toBe(403);

    // —— 权限以服务端为准：打开弹窗后被免职，提交时界面显示 403 的中文提示
    await m.goto(`/t/${secondTopic}`);
    await m.getByRole("button", { name: "管理", exact: true }).click();
    await m.getByRole("menuitem", { name: "置顶" }).click();
    const pin = m.getByRole("dialog", { name: "置顶" });
    await expect(pin).toBeVisible();
    // 超过 5 分钟后再做敏感操作：先弹后台 Passkey 再验证，验证通过才执行
    await admin.clock.fastForward("06:00");
    await row.getByRole("button", { name: "免去" }).click();
    const remove = admin.getByRole("dialog", { name: /^免去/ });
    await remove.getByLabel("理由（写入审计日志）").fill("测试收回权限");
    await remove.getByRole("button", { name: "免去" }).click();
    const reverify = admin.getByRole("dialog", { name: "再次验证后台 Passkey" });
    await reverify.getByRole("button", { name: "用 Passkey 验证" }).click();
    await expect(reverify).toBeHidden();
    await expect(row).toHaveCount(0);
    await pin.getByRole("button", { name: "确认" }).click();
    await expect(pin.getByRole("alert")).toHaveText("没有权限进行此操作");

    // —— 拍卖设置：开关单独确认，参数按小时 / 分钟 / 天填写，超出上下限前端先拦下（刚再验证过，不再弹）
    await admin.goto(`${ADMIN_URL}/auction/`);
    const status = admin.getByText(/^已(开启|关闭)$/);
    const wasOn = (await status.textContent()) === "已开启";
    await admin.getByRole("button", { name: wasOn ? "关闭拍卖" : "开启拍卖" }).click();
    const flip = admin.getByRole("dialog", { name: wasOn ? "关闭拍卖？" : "开启拍卖？" });
    await flip.getByRole("button", { name: wasOn ? "确认关闭" : "确认开启" }).click();
    await expect(flip).toBeHidden();
    await expect(status).toHaveText(wasOn ? "已关闭" : "已开启");
    const params = admin.getByRole("form", { name: "拍卖参数" });
    const payWindow = params.getByLabel("赢家付款期（小时）");
    await payWindow.fill("500");
    await params.getByRole("button", { name: "保存参数" }).click();
    await expect(params).toContainText("不能大于 168 小时");
    await payWindow.fill("36");
    await params.getByRole("button", { name: "保存参数" }).click();
    await expect(admin.getByText("已保存，并已推送管理员群").first()).toBeVisible();
    await admin.reload();
    await expect(
      admin.getByRole("form", { name: "拍卖参数" }).getByLabel("赢家付款期（小时）"),
    ).toHaveValue("36");
    // 还原开关（其他测试按需要自己打开拍卖）
    await admin.getByRole("button", { name: wasOn ? "开启拍卖" : "关闭拍卖" }).click();
    await admin.getByRole("dialog").getByRole("button", { name: /^确认/ }).click();
    await expect(admin.getByText(wasOn ? "已开启" : "已关闭", { exact: true })).toBeVisible();

    // —— 仲裁总览与 Safe 批量文件区块
    await admin.goto(`${ADMIN_URL}/arbitration/`);
    await expect(admin.getByRole("table", { name: "仲裁员工作量" })).toBeVisible();
    await expect(admin.getByRole("heading", { name: "待仲裁 Safe 执行" })).toBeVisible();

    // —— 审计日志：版主的隐藏与站长的任免都在
    await admin.goto(`${ADMIN_URL}/audit/`);
    const audit = admin.getByTestId("audit-row");
    await expect(audit.filter({ hasText: "隐藏主题" }).first()).toContainText(`topic:${ownTopic}`);
    await expect(audit.filter({ hasText: "免职" }).first()).toContainText("测试收回权限");
  } finally {
    await Promise.all([owner.context.close(), mod.context.close(), author.context.close()]);
  }
});
