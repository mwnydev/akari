import { expect, test } from "@playwright/test";
import { db, member } from "./auth";

const CHAIN_ID = 421_614;

/** 订单由 indexer 从链上写入；这里直接在库里造一张「已付款」的一次性担保订单（与后端 order_flow 测试同一写法）。 */
async function seedOrder(buyer: number, seller: number): Promise<number> {
  const sql = db();
  try {
    await sql`INSERT INTO chains (chain_id, name, escrow_address, stream_address, config_address,
                token_address, token_symbol, token_decimals, start_block, safe_depth, finalized_depth)
              VALUES (${CHAIN_ID}, 'e2e', decode(repeat('44', 20), 'hex'), decode(repeat('5e', 20), 'hex'),
                decode(repeat('c0', 20), 'hex'), decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)
              ON CONFLICT (chain_id) DO NOTHING`;
    const chainOrderId = Date.now() * 1000 + Math.floor(Math.random() * 1000);
    const [row] = await sql<{ id: number }[]>`
      INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token,
        amount, instant, offer_hash, listing_hash, state, ship_deadline, level, created_block, updated_block)
      VALUES (${CHAIN_ID}, decode(repeat('44', 20), 'hex'), ${chainOrderId}, decode(repeat('b1', 20), 'hex'),
        decode(repeat('c1', 20), 'hex'), ${buyer}, ${seller}, decode(repeat('33', 20), 'hex'), 45000000, false,
        sha256(${String(chainOrderId)}::bytea), decode(repeat('ab', 32), 'hex'), 1, now() + interval '3 days',
        'latest', 1, 1)
      RETURNING id::int AS id`;
    if (!row) throw new Error("order not created");
    return row.id;
  } finally {
    await sql.end();
  }
}

test("order chat: both parties chat live and see the hash-chain integrity status", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(60_000);
  const base = baseURL ?? "";
  const buyer = await member(browser, base, `buy${info.project.name}`, { username: true });
  const seller = await member(browser, base, `sel${info.project.name}`, { username: true });
  const outsider = await member(browser, base, `out${info.project.name}`);
  try {
    const id = await seedOrder(buyer.id, seller.id);
    await seller.page.goto(`/orders/${id}`);
    await buyer.page.goto(`/orders/${id}`);
    await expect(buyer.page.getByTestId("order-state")).toHaveText("已付款");
    await expect(buyer.page.getByRole("link", { name: "去钱包处理订单" })).toHaveAttribute(
      "href",
      new RegExp(`/order\\?id=${id}$`),
    );
    const buyerChat = buyer.page.getByRole("region", { name: "订单聊天" });
    const sellerChat = seller.page.getByRole("region", { name: "订单聊天" });
    await expect(buyerChat.getByTestId("chain-status")).toContainText("共 0 条");

    await buyerChat.getByLabel("消息内容").fill("卡密收到了吗？我这边显示已付款");
    await buyerChat.getByRole("button", { name: "发送" }).click();
    await expect(sellerChat.getByTestId("chat-message").last()).toContainText("已付款");
    await expect(sellerChat.getByTestId("chat-message").last()).toContainText(buyer.username ?? "");

    // 钱包地址触发「只走担保」提示（服务端标记 warn）
    await sellerChat.getByLabel("消息内容").fill(`直接转我 0x${"ab".repeat(20)}`);
    await sellerChat.getByRole("button", { name: "发送" }).click();
    await expect(buyerChat.getByText("请只通过担保交易付款")).toBeVisible();

    // 哈希链：后端重算整条链通过，条数随新消息更新
    await expect(buyerChat.getByTestId("chain-status")).toContainText("记录完整");
    await expect(buyerChat.getByTestId("chain-status")).toContainText("共 2 条", {
      timeout: 10_000,
    });
    await expect(sellerChat.getByTestId("chat-message").last()).toContainText("已读");

    // 不是买卖双方：订单不存在
    expect((await outsider.page.goto(`/orders/${id}`))?.status()).toBe(404);
  } finally {
    await Promise.all([buyer.context.close(), seller.context.close(), outsider.context.close()]);
  }
});
