import { expect, test, type Page } from "@playwright/test";
import { member } from "./auth";
import { createTopic, replyTo } from "./staff-helpers";

/** 打开主题页并等它订阅上主题推送（WebSocket 回 `subscribed` 且带这个 topic_id）。 */
async function openLive(page: Page, topicId: number) {
  const socket = page.waitForEvent("websocket", (ws) => ws.url().endsWith("/api/v1/ws"));
  await page.goto(`/t/${topicId}`);
  const ws = await socket;
  await ws.waitForEvent("framereceived", {
    predicate: (f) => {
      const text = typeof f.payload === "string" ? f.payload : f.payload.toString();
      return text.includes('"subscribed"') && text.includes(`"topic_id":${topicId}`);
    },
  });
}

// 楼层点赞与朋友圈式评论：一人点赞、评论，另一人不刷新就看到（主题页 WebSocket 推送），刷新后数据还在。
// 主楼（1 楼）只能点赞、不能评论（要说话就回复主题），评论都在 2 楼。
test("likes and comments on a floor show up live for other readers", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const base = baseURL ?? "";
  const a = await member(browser, base, `fsa${info.project.name}`, { username: true });
  const b = await member(browser, base, `fsb${info.project.name}`, { username: true });
  try {
    const title = `楼中评论 ${info.project.name} ${Date.now()}`;
    const topicId = await createTopic(a.page, base, "general", title);
    expect(await replyTo(a.page, base, topicId, "<p>楼主补充</p>")).toBe(2);
    await Promise.all([openLive(a.page, topicId), openLive(b.page, topicId)]);
    const first = b.page.locator("#floor-1");
    await expect(first.getByRole("button", { name: "赞" })).toBeVisible();
    await expect(first.getByRole("button", { name: "评论", exact: true })).toHaveCount(0);
    const floorA = a.page.locator("#floor-2");
    const floorB = b.page.locator("#floor-2");

    // b 点赞：自己立刻变成「已赞」，楼主那边推送更新名单
    await floorB.getByRole("button", { name: "赞" }).click();
    await expect(floorB.getByRole("button", { name: /已赞/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(floorA.getByTestId("floor-social")).toContainText(b.username ?? "");

    // b 评论楼层，a 不刷新就看到
    await floorB.getByRole("button", { name: "评论", exact: true }).click();
    await floorB.getByRole("textbox", { name: "评论 #2" }).fill("什么时候发货？");
    await floorB.getByRole("button", { name: "发送" }).click();
    const fromB = floorA.getByTestId("floor-comment").filter({ hasText: "什么时候发货？" });
    await expect(fromB).toContainText(b.username ?? "");

    // a 点评论即回复评论者
    await fromB.click();
    await floorA.getByRole("textbox", { name: `回复 ${b.username}` }).fill("今晚");
    await floorA.getByRole("button", { name: "发送" }).click();
    const reply = floorB.getByTestId("floor-comment").filter({ hasText: "今晚" });
    await expect(reply).toContainText(`${a.username} 回复 ${b.username}`);

    // b 删除自己的评论，a 那边推送收起
    await floorB
      .getByTestId("floor-comment")
      .filter({ hasText: "什么时候发货？" })
      .getByRole("button", { name: /的评论：更多操作$/ })
      .click();
    await b.page.getByRole("menuitem", { name: "删除", exact: true }).click();
    await expect(fromB).toHaveCount(0);

    // 刷新后：赞与剩下的那条评论都在
    await a.page.reload();
    await expect(floorA.getByRole("button", { name: /^1\s*赞$/ })).toBeVisible();
    await expect(floorA.getByTestId("floor-comment")).toHaveCount(1);
    await expect(floorA.getByTestId("floor-comment")).toContainText("今晚");
  } finally {
    await Promise.all([a.context.close(), b.context.close()]);
  }
});
