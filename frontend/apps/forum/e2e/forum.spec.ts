import { expect, test } from "@playwright/test";
import { backdateAccount, login, uniqueEmail } from "./auth";
import { GUEST_EDGE_CACHE } from "../lib/edge-cache";

// 一页 20 楼：造到 21 楼才有第 2 页
const FLOORS = 21;

test("member creates a topic, replies, edits, deletes and jumps to page 2; guests see the same topic", async ({
  page,
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const email = uniqueEmail(`forum-${info.project.name}`);
  const title = `端到端主题 ${info.project.name} ${Date.now()}`;
  await login(page, email);
  await backdateAccount(email);

  // 发主题：「发新帖」展开底部编辑器，编辑器点开才加载
  await page.goto("/f/general");
  await page.getByRole("button", { name: "发新帖" }).click();
  await page.getByRole("textbox", { name: "标题" }).fill(title);
  await page.getByRole("textbox", { name: "正文" }).fill("一楼正文");
  await page.getByRole("button", { name: "发布主题" }).click();
  await expect(page).toHaveURL(/\/t\/\d+$/);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(title);
  const topicId = Number(new URL(page.url()).pathname.split("/").pop());

  // 回复：点贴底的回复栏展开编辑器；落在当前页，刷新后出现在 2 楼
  await page.getByRole("button", { name: "回复本帖…" }).click();
  await page.getByRole("textbox", { name: "正文" }).fill("二楼原文");
  await page.getByRole("button", { name: "发表回复" }).click();
  const floor2 = page.locator("#floor-2");
  await expect(floor2).toContainText("二楼原文");
  await expect(page.getByRole("button", { name: "回复本帖…" })).toBeVisible();

  // 引用回复：编辑器开头插入引用块，链接回被引用的楼层
  await page.locator("#floor-1").getByRole("button", { name: "引用回复" }).click();
  const composer = page.getByRole("region", { name: "回复编辑器" });
  await expect(composer).toContainText("引用 #1");
  await expect(composer.locator("blockquote")).toContainText("一楼正文");
  await page.keyboard.press("Escape");
  await expect(composer).toBeHidden();

  // 编辑：原地换成编辑器，保存后有编辑历史
  await floor2.getByRole("button", { name: "编辑", exact: true }).click();
  await floor2.getByRole("textbox", { name: "正文" }).fill("二楼改过了");
  await floor2.getByRole("button", { name: "保存修改" }).click();
  await expect(floor2).toContainText("二楼改过了");
  await floor2.getByRole("button", { name: "#2 更多操作" }).click();
  await page.getByRole("menuitem", { name: "编辑历史" }).click();
  const history = page.getByRole("dialog", { name: "#2 编辑历史" });
  await expect(history).toContainText("二楼原文");
  await page.keyboard.press("Escape");
  await expect(history).toBeHidden();

  // 其余楼层直接走 API 造出来（浏览器上下文共用会话 Cookie；写请求必须带白名单 Origin）
  for (let floor = 3; floor <= FLOORS; floor++) {
    const res = await page.request.post(`/api/v1/topics/${topicId}/posts`, {
      headers: { origin: new URL(baseURL ?? page.url()).origin },
      data: { html: `<p>第 ${floor} 楼</p>`, source: { type: "doc" } },
    });
    expect(res.status(), await res.text()).toBe(201);
  }

  // 删除自己的楼层：楼层号保留
  await page.reload();
  const floor3 = page.locator("#floor-3");
  await floor3.getByRole("button", { name: "#3 更多操作" }).click();
  await page.getByRole("menuitem", { name: "删除" }).click();
  await page.getByRole("button", { name: "确认删除" }).click();
  await expect(floor3).toContainText("该楼层已被删除");

  // 按楼层跳页
  await page.getByLabel("跳到", { exact: true }).fill(String(FLOORS));
  await page.getByRole("button", { name: "跳转" }).click();
  await expect(page).toHaveURL(new RegExp(`/t/${topicId}\\?page=2#floor-${FLOORS}$`));
  await expect(page.locator(`#floor-${FLOORS}`)).toContainText(`第 ${FLOORS} 楼`);
  await expect(page.getByText("第 2 / 2 页")).toBeVisible();
  await page.getByRole("navigation", { name: "分页" }).getByRole("link", { name: "1" }).click();
  await expect(page).toHaveURL(new RegExp(`/t/${topicId}$`));

  // 游客：同一个主题，可缓存，没有任何编辑入口
  const guest = await browser.newContext({ baseURL: baseURL ?? "" });
  try {
    const gp = await guest.newPage();
    const res = await gp.goto(`/t/${topicId}`);
    expect(res?.headers()["cloudflare-cdn-cache-control"]).toBe(GUEST_EDGE_CACHE);
    await expect(gp.getByRole("heading", { level: 1 })).toHaveText(title);
    await expect(gp.locator("#floor-2")).toContainText("二楼改过了");
    await expect(gp.locator("#floor-3")).toContainText("该楼层已被删除");
    await expect(gp.getByRole("button", { name: "编辑", exact: true })).toHaveCount(0);
    await gp.locator("#floor-2").getByRole("button", { name: "#2 更多操作" }).click();
    await expect(gp.getByRole("menuitem", { name: "复制楼层链接" })).toBeVisible();
    await expect(gp.getByRole("menuitem", { name: "删除" })).toHaveCount(0);
    await gp.keyboard.press("Escape");
    await expect(gp.getByText("后参与回复")).toBeVisible();
    await gp.goto(`/t/${topicId}?page=2`);
    await expect(gp.locator(`#floor-${FLOORS}`)).toBeVisible();
  } finally {
    await guest.close();
  }
});

test("unknown topics and boards render the Chinese not-found page", async ({ page }) => {
  const res = await page.goto("/t/999999999");
  expect(res?.status()).toBe(404);
  await expect(page.getByText("内容不存在或已被删除")).toBeVisible();
  expect((await page.goto("/f/no-such-board"))?.status()).toBe(404);
});
