import { expect, test, type Page } from "@playwright/test";
import { member } from "./auth";
import { createTopic, forumId, grantRole } from "./staff-helpers";

// 举报处理与楼层隐藏（ADR 0013）：用户举报 → 版主在「举报处理」里隐藏楼层并结案 → 举报人只收到「已处理」的通知；
// 版主在楼层菜单里隐藏 / 取消隐藏，作者看得到内容与理由、其他人只看到「已隐藏」。
// 需要 forum-worker 在跑（结案通知从发件箱写入站内通知），板块种子数据见 e2e/README.md。

/** 经同源 API 回一楼，返回楼层号。 */
async function replyTo(page: Page, baseURL: string, topic: number, text: string) {
  const res = await page.request.post(`/api/v1/topics/${topic}/posts`, {
    headers: { origin: new URL(baseURL).origin },
    data: { html: `<p>${text}</p>`, source: { type: "doc" } },
  });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { floor: number }).floor;
}

function floor(page: Page, n: number) {
  return page.locator(`#floor-${n}`);
}

async function floorMenu(page: Page, n: number, item: string) {
  await page.getByRole("button", { name: `#${n} 更多操作` }).click();
  await page.getByRole("menuitem", { name: item }).click();
}

test("a reported floor is hidden from the report queue and the reporter is told", async ({
  browser,
  baseURL,
}, info) => {
  test.skip(info.project.name !== "desktop", "管理流程只跑桌面");
  test.setTimeout(90_000);
  const base = baseURL ?? "";
  const author = await member(browser, base, "rauthor", { username: true });
  const reporter = await member(browser, base, "reporter", { username: true });
  const mod = await member(browser, base, "rmod", { username: true });
  try {
    await grantRole(mod.id, 4, await forumId("general"));
    const topic = await createTopic(author.page, base, "general", "举报处理的主题");
    // 内容带时间戳：队列里可能有之前运行留下的案件
    const text = `加我私聊低价出号 ${Date.now()}`;
    const n = await replyTo(author.page, base, topic, text);

    // 举报人在楼层菜单里举报
    const r = reporter.page;
    await r.goto(`/t/${topic}`);
    await floorMenu(r, n, "举报");
    await expect(floor(r, n).getByRole("status")).toHaveText("已举报，管理人员会尽快处理");

    // 版主从账号菜单进「举报处理」
    const m = mod.page;
    await m.goto("/");
    await m.getByTestId("nav-user").click();
    await m.getByRole("link", { name: "举报处理", exact: true }).click();
    await expect(m.getByRole("heading", { name: "举报处理" })).toBeVisible();
    const item = m.getByTestId("report-case").filter({ hasText: text });
    await expect(item).toContainText("1 人举报");
    // 版主看不到举报人是谁
    await expect(item).not.toContainText(reporter.username ?? "");
    await item.getByRole("button", { name: "隐藏" }).click();
    const dialog = m.getByRole("dialog", { name: `隐藏 #${n}` });
    await dialog.locator('select[name="preset"]').selectOption("广告或引流");
    await dialog.getByRole("button", { name: "隐藏并结案" }).click();
    await expect(dialog).toBeHidden();
    await expect(item).toHaveCount(0);
    await m.getByRole("link", { name: "已处理" }).click();
    await expect(
      m.getByTestId("report-case").filter({ hasText: text }).getByTestId("report-outcome"),
    ).toContainText("已处理（隐藏）");

    // 举报人：楼层只显示「已隐藏」，并收到只说结果的通知
    await r.goto(`/t/${topic}`);
    await expect(floor(r, n)).toContainText("该楼层已被管理人员隐藏");
    await expect(floor(r, n)).not.toContainText(text);
    await expect(async () => {
      await r.goto("/notifications");
      await expect(r.getByText(`你举报的内容（#${n} 楼）已处理，感谢反馈`)).toBeVisible({
        timeout: 1000,
      });
    }).toPass({ timeout: 20_000 });

    // 作者：看得到自己的内容与隐藏理由
    await author.page.goto(`/t/${topic}`);
    await expect(floor(author.page, n).getByTestId("hidden-notice")).toHaveText(
      "这层楼已被管理人员隐藏，其他人看不到。理由：广告或引流",
    );
    await expect(floor(author.page, n)).toContainText(text);
  } finally {
    await Promise.all([author, reporter, mod].map((u) => u.context.close()));
  }
});

test("a moderator hides and unhides a floor from its menu", async ({ browser, baseURL }, info) => {
  test.skip(info.project.name !== "desktop", "管理流程只跑桌面");
  test.setTimeout(60_000);
  const base = baseURL ?? "";
  const author = await member(browser, base, "hauthor", { username: true });
  const mod = await member(browser, base, "hmod", { username: true });
  const guest = await browser.newContext({ baseURL: base });
  try {
    await grantRole(mod.id, 4, await forumId("general"));
    const topic = await createTopic(author.page, base, "general", "楼层隐藏的主题");
    const n = await replyTo(author.page, base, topic, "需要隐藏的楼层内容");

    const m = mod.page;
    await m.goto(`/t/${topic}`);
    await floorMenu(m, n, "隐藏楼层");
    const hide = m.getByRole("dialog", { name: `隐藏 #${n}` });
    await hide.locator('select[name="preset"]').selectOption("其他");
    // 理由必填：选「其他」不写说明不能提交
    await hide.getByRole("button", { name: "隐藏", exact: true }).click();
    await expect(hide.getByText("选「其他」时请写明原因")).toBeVisible();
    await hide.getByLabel("补充说明（选「其他」时必填）").fill("含联系方式");
    await hide.getByRole("button", { name: "隐藏", exact: true }).click();
    await expect(hide).toBeHidden();
    await expect(floor(m, n).getByTestId("hidden-notice")).toHaveText(
      "已隐藏，只有管理人员和作者看得到。理由：其他：含联系方式",
    );

    // 游客只看到「已隐藏」，没有点赞与评论
    const g = await guest.newPage();
    await g.goto(`/t/${topic}`);
    await expect(floor(g, n)).toContainText("该楼层已被管理人员隐藏");
    await expect(floor(g, n)).not.toContainText("需要隐藏的楼层内容");

    // 取消隐藏
    await floorMenu(m, n, "取消隐藏");
    const unhide = m.getByRole("dialog", { name: `取消隐藏 #${n}` });
    await unhide.getByRole("button", { name: "取消隐藏" }).click();
    await expect(unhide).toBeHidden();
    await expect(floor(m, n).getByTestId("hidden-notice")).toHaveCount(0);
    await g.reload();
    await expect(floor(g, n)).toContainText("需要隐藏的楼层内容");
  } finally {
    await Promise.all([author.context.close(), mod.context.close(), guest.close()]);
  }
});
