import { expect, test } from "@playwright/test";
import { db, member } from "./auth";

test("two members message each other live: notification bell, unread, read receipts, typing, blocking", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const base = baseURL ?? "";
  const a = await member(browser, base, `dma${info.project.name}`, { username: true });
  const b = await member(browser, base, `dmb${info.project.name}`, { username: true });
  try {
    // B 停在首页，导航栏的通知角标靠推送更新
    await b.page.goto("/");
    const bBadge = b.page.getByTestId("unread-badge");
    await expect(bBadge).toHaveCount(0);

    // A 从 B 的主页（按 ID 打开会换成用户名地址）发起私信
    await a.page.goto(`/u/${b.id}`);
    await expect(a.page).toHaveURL(new RegExp(`/u/${b.username}$`));
    await a.page.getByRole("button", { name: "发私信" }).click();
    await expect(a.page).toHaveURL(/\/messages\/\d+$/);
    const conversationUrl = a.page.url();
    await a.page.getByLabel("消息内容").fill("你好，在吗？");
    await a.page.getByRole("button", { name: "发送" }).click();
    const aChat = a.page.getByRole("region", { name: `与 ${b.username} 的私信` });
    await expect(aChat.getByTestId("chat-message").last()).toContainText("你好，在吗？");

    // B：不刷新页面，通知角标出现；会话列表显示未读
    await expect(bBadge).toHaveText("1", { timeout: 15_000 });
    await b.page.goto("/messages");
    const conv = b.page.getByTestId("conversation").first();
    await expect(conv).toContainText(a.username ?? "");
    await expect(conv.getByTestId("unread")).toHaveText("1");
    await conv.click();
    const bChat = b.page.getByRole("region", { name: `与 ${a.username} 的私信` });
    await expect(bChat.getByTestId("chat-message").last()).toContainText("你好，在吗？");

    // A 看到已读回执；B 输入时 A 看到「正在输入」，B 发出的回复实时出现在 A 的会话里
    await expect(aChat.getByTestId("chat-message").last()).toContainText("已读");
    await bChat.getByLabel("消息内容").fill("在的");
    await expect(aChat.getByText("对方正在输入…")).toBeVisible({ timeout: 10_000 });
    await bChat.getByRole("button", { name: "发送" }).click();
    await expect(aChat.getByTestId("chat-message").last()).toContainText("在的");
    await expect(aChat.getByText("对方正在输入…")).toBeHidden();

    // 通知页：B 的私信通知可点回会话，标为已读后角标消失
    await b.page.goto("/notifications");
    const note = b.page.getByTestId("notification").first();
    await expect(note).toContainText(`${a.username} 给你发了私信`);
    await expect(note).toHaveAttribute("data-unread", "true");
    await b.page.getByRole("button", { name: "全部标为已读" }).click();
    await expect(note).not.toHaveAttribute("data-unread", "true");
    await expect(b.page.getByTestId("unread-badge")).toHaveCount(0);

    // B 屏蔽 A：B 不能再发，A 发送时看到中文错误
    await b.page.goto(conversationUrl);
    await b.page.getByRole("button", { name: "屏蔽", exact: true }).click();
    await b.page.getByRole("button", { name: "确认屏蔽" }).click();
    await expect(b.page.getByText("你已屏蔽对方，解除屏蔽后才能互发私信")).toBeVisible();
    await aChat.getByLabel("消息内容").fill("还在吗");
    await aChat.getByRole("button", { name: "发送" }).click();
    await expect(aChat.getByRole("alert")).toContainText("屏蔽");
    await b.page.getByRole("button", { name: "解除屏蔽" }).click();
    await expect(b.page.getByLabel("消息内容")).toBeVisible();
  } finally {
    await a.context.close();
    await b.context.close();
  }
});

test("tabs of one browser share a single WebSocket and all get live updates", async ({
  browser,
  baseURL,
}, info) => {
  const base = baseURL ?? "";
  const a = await member(browser, base, `taba${info.project.name}`, { username: true });
  const b = await member(browser, base, `tabb${info.project.name}`);
  try {
    let sockets = 0;
    a.context.on("page", (p) => p.on("websocket", () => sockets++));
    a.page.on("websocket", () => sockets++);
    await a.page.goto("/");
    const second = await a.context.newPage();
    await second.goto("/");
    // 第一个标签页持有连接；第二个经 BroadcastChannel 收事件
    await expect.poll(() => sockets).toBe(1);

    await b.page.goto(`/u/${a.id}`);
    await b.page.getByRole("button", { name: "发私信" }).click();
    await b.page.getByLabel("消息内容").fill("两个标签页都该看到");
    await b.page.getByRole("button", { name: "发送" }).click();
    await expect(a.page.getByTestId("unread-badge")).toHaveText("1", { timeout: 15_000 });
    await expect(second.getByTestId("unread-badge")).toHaveText("1");
    expect(sockets).toBe(1);

    // 持有连接的标签页关掉后，另一个接手（新开一条连接）
    await a.page.close();
    await expect.poll(() => sockets, { timeout: 10_000 }).toBe(2);
  } finally {
    await a.context.close();
    await b.context.close();
  }
});

test("without a working WebSocket the conversation falls back to polling", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(60_000);
  const base = baseURL ?? "";
  const a = await member(browser, base, `pola${info.project.name}`, { username: true });
  const b = await member(browser, base, `polb${info.project.name}`, { username: true });
  try {
    // B 的浏览器里每次握手都被立即关闭（如代理不支持 WebSocket）
    await b.context.routeWebSocket(/\/api\/v1\/ws$/, (ws) => ws.close());
    await a.page.goto(`/u/${b.id}`);
    await a.page.getByRole("button", { name: "发私信" }).click();
    await a.page.getByLabel("消息内容").fill("第一条");
    await a.page.getByRole("button", { name: "发送" }).click();
    const conversation = a.page.url().replace(base, "");

    await b.page.goto(conversation);
    const bChat = b.page.getByRole("region", { name: `与 ${a.username} 的私信` });
    await expect(bChat.getByTestId("chat-message").last()).toContainText("第一条");
    await a.page.getByLabel("消息内容").fill("轮询也能收到");
    await a.page.getByRole("button", { name: "发送" }).click();
    // 连续 3 次连不上（退避约 0.5 + 1 + 2 秒）后每 5 秒补拉一次
    await expect(bChat.getByTestId("chat-message").last()).toContainText("轮询也能收到", {
      timeout: 25_000,
    });
  } finally {
    await a.context.close();
    await b.context.close();
  }
});

/** 报价卡片由卖家在钱包里签名后提交；这里直接在库里造一张指定买家的卡片和对应的会话消息。 */
async function seedOffer(conversation: number, seller: number, buyer: number) {
  const sql = db();
  try {
    const [offer] = await sql<{ id: number }[]>`
      INSERT INTO offers (conversation_id, seller_id, buyer_id, chain_id, verifying_contract, seller, buyer,
        token, amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant,
        ship_window, confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, expires_at)
      VALUES (${conversation}, ${seller}, ${buyer}, 421614, decode(repeat('44', 20), 'hex'),
        decode(repeat('c1', 20), 'hex'), decode(repeat('b1', 20), 'hex'), decode(repeat('33', 20), 'hex'),
        45000000, 0, 100, 100000, 1000000, 0, 0, false, 259200, 259200, decode(repeat('ab', 32), 'hex'),
        extract(epoch FROM now() + interval '1 day')::bigint, 1, ${Date.now()}, decode('00', 'hex'),
        sha256(${`offer-${conversation}-${Date.now()}`}::bytea), now() + interval '1 day')
      RETURNING id::int AS id`;
    if (!offer) throw new Error("offer not created");
    await sql`WITH m AS (
                INSERT INTO messages (conversation_id, sender_id, kind, body, offer_id)
                VALUES (${conversation}, ${seller}, 'offer', 'created', ${offer.id}) RETURNING id)
              UPDATE conversations SET last_message_id = (SELECT id FROM m) WHERE id = ${conversation}`;
    return offer.id;
  } finally {
    await sql.end();
  }
}

test("offer cards in a conversation: the buyer can pay in the wallet or reject; the seller sees it live", async ({
  browser,
  baseURL,
}, info) => {
  const base = baseURL ?? "";
  const seller = await member(browser, base, `ofs${info.project.name}`, { username: true });
  const buyer = await member(browser, base, `ofb${info.project.name}`, { username: true });
  try {
    const res = await seller.page.request.post("/api/v1/conversations", {
      headers: { origin: new URL(base).origin },
      data: { user_id: buyer.id },
    });
    const { id: conversation } = (await res.json()) as { id: number };
    const offerId = await seedOffer(conversation, seller.id, buyer.id);

    await seller.page.goto(`/messages/${conversation}`);
    const sellerCard = seller.page.getByTestId("offer-message").last();
    await expect(sellerCard).toContainText("发出了担保报价");
    // 卖家看自己的卡片：可取消，没有购买按钮；发起担保去钱包
    await expect(sellerCard.getByRole("button", { name: "取消报价" })).toBeVisible();
    await expect(sellerCard.getByRole("link", { name: "担保购买" })).toHaveCount(0);
    await expect(seller.page.getByRole("link", { name: "发起担保" })).toHaveAttribute(
      "href",
      new RegExp(`/offer\\?conversation=${conversation}$`),
    );

    await buyer.page.goto(`/messages/${conversation}`);
    const buyerCard = buyer.page.getByTestId("offer-message").last();
    await expect(buyerCard.getByRole("link", { name: "担保购买" })).toHaveAttribute(
      "href",
      new RegExp(`/pay\\?offer=${offerId}$`),
    );
    await buyerCard.getByRole("button", { name: "拒绝报价" }).click();
    await expect(buyer.page.getByTestId("offer-message").first()).toContainText("已拒绝");
    await expect(seller.page.getByTestId("offer-message").last()).toContainText("拒绝了担保报价");
  } finally {
    await seller.context.close();
    await buyer.context.close();
  }
});
