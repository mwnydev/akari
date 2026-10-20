import { expect, test } from "@playwright/test";
import { backdateAccount, login, member, uniqueEmail } from "./auth";
import { createTopic, replyTo } from "./staff-helpers";
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

  // 发主题：「发新帖」打开编辑器弹窗，编辑器点开才加载
  await page.goto("/f/general");
  await page.getByRole("button", { name: "发新帖" }).click();
  await page.getByRole("textbox", { name: "标题" }).fill(title);
  await page.getByRole("textbox", { name: "正文" }).fill("一楼正文");
  await page.getByRole("button", { name: "发布主题" }).click();
  await expect(page).toHaveURL(/\/t\/\d+$/);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(title);
  const topicId = Number(new URL(page.url()).pathname.split("/").pop());

  // 回复：点贴底的回复栏打开编辑器弹窗；落在当前页，刷新后出现在 2 楼；主楼没有「评论」
  await expect(
    page.locator("#floor-1").getByRole("button", { name: "评论", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "回复本帖…" }).click();
  await expect(page.getByRole("dialog", { name: "回复本帖" })).toBeVisible();
  await page.getByRole("textbox", { name: "正文" }).fill("二楼原文");
  await page.getByRole("button", { name: "发表回复" }).click();
  const floor2 = page.locator("#floor-2");
  await expect(floor2).toContainText("二楼原文");
  await expect(page.getByRole("button", { name: "回复本帖…" })).toBeVisible();

  // 引用：弹窗标题写明引用谁，编辑器开头插入引用块，链接回被引用的楼层；没改动时 Esc 直接关闭
  await page.locator("#floor-1").getByRole("button", { name: "引用", exact: true }).click();
  const composer = page.getByRole("dialog", { name: /^引用 #1 / });
  await expect(composer.locator("blockquote")).toContainText("一楼正文");
  await page.keyboard.press("Escape");
  await expect(composer).toBeHidden();

  // 写了内容再关：先确认（内容已存为草稿），继续编辑则留在弹窗里
  await page.locator("#floor-2").getByRole("button", { name: "回复", exact: true }).click();
  const replying = page.getByRole("dialog", { name: /^回复 #2 / });
  await replying.getByRole("textbox", { name: "正文" }).pressSequentially("还没写完");
  await replying.getByRole("button", { name: "关闭", exact: true }).click();
  await replying.getByRole("button", { name: "继续编辑" }).click();
  await expect(replying.getByRole("alertdialog")).toBeHidden();
  await replying.getByRole("button", { name: "关闭", exact: true }).click();
  await replying.getByRole("button", { name: "仍然关闭" }).click();
  await expect(replying).toBeHidden();
  // 草稿留在本机：从回复栏打开时接着写，放弃草稿后才清掉
  await page.getByRole("button", { name: "回复本帖…" }).click();
  const draft = page.getByRole("dialog", { name: "回复本帖" });
  await expect(draft).toContainText("还没写完");
  await draft.getByRole("button", { name: "放弃草稿" }).click();
  await expect(draft).toBeHidden();

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

test("quoting a floor keeps only its own words, and replying to a floor @-mentions its author", async ({
  browser,
  baseURL,
}, info) => {
  const base = baseURL ?? "";
  const a = await member(browser, base, `qa${info.project.name}`, { username: true });
  const b = await member(browser, base, `qb${info.project.name}`, { username: true });
  try {
    const topicId = await createTopic(a.page, base, "general", `引用不嵌套 ${Date.now()}`);
    await replyTo(
      a.page,
      base,
      topicId,
      "<blockquote><p>#1 楼主：一楼原话</p></blockquote><p>二楼自己的话</p>",
    );
    await b.page.goto(`/t/${topicId}`);
    await b.page.locator("#floor-2").getByRole("button", { name: "引用", exact: true }).click();
    const quoting = b.page.getByRole("dialog", { name: `引用 #2 ${a.username}` });
    const quote = quoting.locator("blockquote");
    await expect(quote).toContainText("二楼自己的话");
    await expect(quote).not.toContainText("一楼原话");
    await b.page.keyboard.press("Escape");
    await expect(quoting).toBeHidden();

    await b.page.locator("#floor-2").getByRole("button", { name: "回复", exact: true }).click();
    const replying = b.page.getByRole("dialog", { name: `回复 #2 ${a.username}` });
    await expect(replying.getByRole("textbox", { name: "正文" })).toContainText(`@${a.username}`);
  } finally {
    await Promise.all([a.context.close(), b.context.close()]);
  }
});

test("unknown topics and boards render the Chinese not-found page", async ({ page }) => {
  const res = await page.goto("/t/999999999");
  expect(res?.status()).toBe(404);
  await expect(page.getByText("内容不存在或已被删除")).toBeVisible();
  expect((await page.goto("/f/no-such-board"))?.status()).toBe(404);
});
