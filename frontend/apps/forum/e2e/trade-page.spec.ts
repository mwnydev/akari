import { expect, test } from "@playwright/test";
import { db, member } from "./auth";

/**
 * 交易帖由钱包签名后创建；这里先经 API 发一个普通主题，再直接在库里把它变成交易帖（与 messaging.spec 造报价同一思路）。
 */
async function seedListing(topicId: number, sellerId: number): Promise<number> {
  const sql = db();
  try {
    await sql`UPDATE topics SET kind = 1 WHERE id = ${topicId}`;
    const [row] = await sql<{ id: number }[]>`
      INSERT INTO trade_listings (topic_id, seller_id, chain_id, price, terms, content_hash, cny_per_usdt)
      VALUES (${topicId}, ${sellerId}, 421614, 45000000,
        ${sql.json({ instant: true, ship_window: 0, confirm_window: 259200, bond: "10000000", fee_payer: 0 })},
        sha256(${`e2e-listing-${topicId}`}::bytea), 72000)
      RETURNING id::int AS id`;
    if (!row) throw new Error("listing not created");
    return row.id;
  } finally {
    await sql.end();
  }
}

test("trade listing: price panel with ¥ reference, list row links to it, 发起担保 opens the wallet offer flow", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(60_000);
  const base = baseURL ?? "";
  const origin = new URL(base).origin;
  const seller = await member(browser, base, `trs${info.project.name}`, { username: true });
  const buyer = await member(browser, base, `trb${info.project.name}`, { username: true });
  try {
    const title = `担保交易测试 ${info.project.name} ${Date.now()}`;
    const res = await seller.page.request.post("/api/v1/forums/trade/topics", {
      headers: { origin },
      data: { title, html: "<p>自动发卡，现货</p>", source: { type: "doc" } },
    });
    expect(res.status(), await res.text()).toBe(201);
    const { topic_id: topicId } = (await res.json()) as { topic_id: number };
    const listingId = await seedListing(topicId, seller.id);

    // 板块列表：交易帖带「担保交易」标签与标价，链接到交易帖页面；「交易」筛选只留交易帖
    const page = buyer.page;
    await page.goto("/f/trade");
    const row = page.getByTestId("topic-row").filter({ hasText: title });
    await expect(row).toContainText("担保交易");
    await expect(row).toContainText("45.00");
    await page.getByRole("radio", { name: "交易" }).click();
    await expect(row).toBeVisible();
    await row.getByRole("link", { name: title }).click();
    await expect(page).toHaveURL(new RegExp(`/trade/${listingId}$`));

    // 价格面板：USDT 标价、人民币参考价、发货方式、确认期、保证金、担保说明
    const panel = page.getByRole("complementary", { name: "价格与条款" });
    await expect(panel).toContainText("45.00");
    await expect(panel).toContainText("≈ ¥324.00");
    await expect(panel).toContainText("自动发卡");
    await expect(panel).toContainText("收货后 3 天");
    await expect(panel).toContainText("资金托管在合约里");
    // 还没有公开报价：担保购买不可点
    await expect(panel.getByRole("button", { name: "担保购买" })).toBeDisabled();

    // 发起担保：先打开与卖家的私信会话，再去钱包在会话里提出报价（与私信页同一个入口）
    await page.route("**/offer?conversation=*", (route) =>
      route.fulfill({ contentType: "text/html", body: "<p>wallet</p>" }),
    );
    await panel.getByRole("button", { name: "发起担保" }).click();
    await expect(page).toHaveURL(/\/offer\?conversation=\d+$/);

    // 卖家本人：去钱包发布公开报价
    await seller.page.goto(`/trade/${listingId}`);
    await expect(seller.page.getByRole("link", { name: "发布公开报价" })).toHaveAttribute(
      "href",
      new RegExp(`/offer\\?listing=${listingId}$`),
    );

    // 游客：可缓存，操作按钮换成登录链接
    const guest = await browser.newContext({ baseURL: base });
    try {
      const gp = await guest.newPage();
      const r = await gp.goto(`/trade/${listingId}`);
      expect(r?.headers()["cloudflare-cdn-cache-control"]).toBe("public, max-age=60");
      await expect(gp.getByRole("link", { name: "私信卖家" })).toHaveAttribute(
        "href",
        /\/login\?next=/,
      );
    } finally {
      await guest.close();
    }
  } finally {
    await seller.context.close();
    await buyer.context.close();
  }
});

test("home: latest / hot / boards tabs; the sidebar lists boards from the API", async ({
  page,
}) => {
  await page.goto("/");
  const tabs = page.getByRole("tablist", { name: "主题排序" });
  await expect(tabs.getByRole("tab", { name: "最新" })).toHaveAttribute("aria-selected", "true");
  await tabs.getByRole("tab", { name: "板块" }).click();
  await expect(page.getByRole("table", { name: "全部板块" })).toContainText("综合讨论");

  await page.goto("/?tab=hot");
  await expect(tabs.getByRole("tab", { name: "热门" })).toHaveAttribute("aria-selected", "true");

  // 侧栏：桌面常驻，窄屏收进汉堡按钮打开的抽屉
  const open = page.getByRole("button", { name: "打开导航" });
  if (await open.isVisible()) await open.click();
  const nav = page.getByRole("navigation", { name: "板块" }).filter({ visible: true });
  await nav.getByRole("link", { name: "技术" }).click();
  await expect(page).toHaveURL(/\/f\/tech$/);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("技术");
});
