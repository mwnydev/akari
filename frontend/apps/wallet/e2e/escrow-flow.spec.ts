import { escrowAbi, localChain as chain } from "@akari/contracts";
import { expect, test, type Page } from "@playwright/test";
import { backendPaths, chainIsDown, publicClient } from "./env";
import { ensureBoard, signIn } from "./forum-user";
import {
  U,
  balances,
  confirmSignature,
  createWallet,
  harness,
  mintUsdt,
  withdrawCredits,
} from "./harness";

// 完整担保流程：两个浏览器上下文各带一个 CDP 虚拟 Passkey（卖家、买家），链上余额按前后差额断言。
// 前置：deploy/dev/aa.sh up。链或 bundler 不在线时跳过。
//
// 后端按接口逐个决定：WALLET_E2E_API 上的 forum-api 实现了的接口走真实后端（登录、邮件验证码经 Mailpit），
// 没实现的（或后端没运行）由 ./fake-backend.ts 在浏览器网络层模拟；没有 /aa/rpc 时用 DEV 代付
// （playwright.config.ts 同样据此决定）。全部接口都在时就是完整的真实流程。

/** 用卖家的会话在测试版块发一个 20 USDT 的交易帖（卖家承担手续费），返回交易帖 ID。 */
async function createListing(page: Page): Promise<number> {
  const slug = ensureBoard();
  const res = await page.evaluate(async (slug) => {
    const r = await fetch(`/api/v1/forums/${slug}/listings`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        title: "e2e 测试商品",
        html: "<p>e2e 测试商品，九成新</p>",
        source: { type: "doc" },
        price: "20000000",
        terms: { ship_window: 3 * 86_400, confirm_window: 7 * 86_400 },
      }),
    });
    return { status: r.status, body: (await r.json()) as { listing_id?: number } };
  }, slug);
  expect(res.status, JSON.stringify(res.body)).toBe(201);
  return res.body.listing_id ?? 0;
}

/** 订单接口（站内 ID）→ 链上订单号；走页面的 fetch，模拟后端也能接住。 */
async function chainOrderId(page: Page, orderId: string): Promise<bigint> {
  const order = await page.evaluate(
    async (id) =>
      (await fetch(`/api/v1/orders/${id}`)).json() as Promise<{ chain_order_id: number }>,
    orderId,
  );
  return BigInt(order.chain_order_id);
}

test("开通交易 → 卖家发报价 → 买家一次 Face ID 付款 → 卖家发货 → 买家确认收货", async ({
  browser,
  baseURL,
}) => {
  const down = await chainIsDown();
  test.skip(down !== null, down ?? "");
  const h = await harness();
  const base = baseURL ?? "";

  const seller = await h.newUser(browser, "seller", base);
  const buyer = await h.newUser(browser, "buyer", base);
  const sellerAddr = await createWallet(seller);
  const buyerAddr = await createWallet(buyer);
  expect(sellerAddr).not.toBe(buyerAddr);
  // 只算出地址，还没上链
  expect(await publicClient.getCode({ address: sellerAddr })).toBeUndefined();

  await mintUsdt(buyerAddr, U("50"));

  const feeRecipient = await publicClient.readContract({
    address: chain.contracts.Escrow,
    abi: escrowAbi,
    functionName: "feeRecipient",
  });
  const parties = {
    buyer: buyerAddr,
    seller: sellerAddr,
    escrow: chain.contracts.Escrow,
    paymaster: chain.contracts.FixedFeePaymaster,
    feeRecipient,
  };
  /** 各方 USDT 余额相对开始时的变化（同一条链可以反复跑，只看差额）。 */
  const before = await balances(parties);
  const changes = async () => {
    const now = await balances(parties);
    const names = Object.keys(parties) as (keyof typeof parties)[];
    return Object.fromEntries(names.map((k) => [k, now[k] - before[k]]));
  };

  // 卖家：为自己的交易帖签公开报价，或在私信会话里签一份只给这位买家的报价，都是 20 USDT
  // （钱包未部署 → ERC-6492 签名，页面用 eth_call 验证）。注册 Passkey 刚算过一次近期验证，
  // 所以这里只有签报价这一次 Face ID。
  const sellerSigns = await seller.signCount();
  if (h.realListings) {
    await seller.page.goto(`/offer?listing=${await createListing(seller.page)}`);
  } else {
    const conversation = 1 + Math.floor(Math.random() * 1e9);
    await seller.page.goto(`/offer?conversation=${conversation}`);
    await seller.page.getByLabel("商品 / 服务").fill("e2e 测试商品");
    await seller.page.getByLabel("金额（USDT）").fill("20");
    await seller.page.getByLabel("买家钱包地址").fill(buyerAddr);
  }
  await seller.page.getByRole("button", { name: "签名报价" }).click();
  await confirmSignature(seller.page, "签署担保报价", "20.00");
  await expect(seller.page.getByTestId("offer-verified")).toBeVisible();
  expect(await seller.signCount()).toBe(sellerSigns + 1);
  const link = (await seller.page.getByTestId("offer-link").textContent()) ?? "";
  expect(link).toMatch(/\/pay\?offer=\d+$/);
  const offerId = new URL(link).searchParams.get("offer") ?? "";

  // 买家：approve + createAndFund 一个 UserOperation（卖家承担手续费，买家付 20 + 0.05 代付费）
  await buyer.page.goto(link);
  await expect(buyer.page.getByTestId("pay-total")).toContainText("20.05");
  const buyerSigns = await buyer.signCount();
  await buyer.page.getByRole("button", { name: /^付款/ }).click();
  await confirmSignature(buyer.page, "担保付款", "20.05");
  expect(await buyer.signCount()).toBe(buyerSigns + 1); // 授权 + 付款只刷一次 Face ID
  // 付款结果页等 indexer 同步出订单（GET /offers/{id}/order）才给出订单链接：站内订单 ID，不是链上订单号
  const orderHref = (await buyer.page.getByTestId("order-link").getAttribute("href")) ?? "";
  const orderId = new URL(orderHref, "http://x").searchParams.get("id") ?? "";
  expect(Number(orderId)).toBeGreaterThan(0);
  if (!h.fakeOrders) {
    // indexer 看到成交后把报价卡片标为已付款（只能成交一次）
    const offer = await seller.page.evaluate(
      async (id) => (await fetch(`/api/v1/offers/${id}`)).json() as Promise<{ status: string }>,
      offerId,
    );
    expect(offer.status).toBe("paid");
  }

  expect(await changes()).toEqual({
    buyer: -U("20.05"),
    seller: 0n,
    escrow: U("20"), // 托管
    paymaster: U("0.05"), // 验证阶段收的代付费
    feeRecipient: 0n,
  });
  expect(await publicClient.getCode({ address: buyerAddr })).toBeDefined(); // 首次操作顺带部署

  // 卖家：订单页（indexer 数据，自动刷新）标记发货（付款后的操作免代付费；卖家钱包这时才部署）
  await seller.page.goto(`/order?id=${orderId}`);
  await expect(seller.page.getByTestId("order-level")).toHaveText("已上链（处理中）");
  await seller.page.getByRole("button", { name: "标记已发货" }).click();
  await confirmSignature(seller.page, "标记已发货", "20.00");
  await expect(seller.page.getByTestId("order-state")).toHaveText("卖家已发货，等待买家确认收货");
  await expect(seller.page.getByRole("button", { name: "标记已发货" })).toBeHidden();

  // 买家：订单页不用刷新，看到卖家已发货后确认收货 → 放款
  await buyer.page.goto(`/order?id=${orderId}`);
  await expect(buyer.page.getByTestId("order-state")).toHaveText("卖家已发货，等待买家确认收货");
  await buyer.page.getByRole("button", { name: "确认收货" }).click();
  await confirmSignature(buyer.page, "确认收货", "20.00");
  await expect(buyer.page.getByTestId("order-state")).toHaveText("已确认收货，货款已放给卖家");
  // 卖家那边的页面自己刷新出最终状态
  await expect(seller.page.getByTestId("order-state")).toHaveText("已确认收货，货款已放给卖家");
  await expect(buyer.page.getByRole("button", { name: "确认收货" })).toBeHidden();
  await expect(buyer.page.getByTestId("order-level")).toHaveText("已上链（处理中）");
  // 免费代付提币次数：开通送 1 次；订单已结束但还没最终确认，不发
  if (h.realSponsor) {
    for (const u of [buyer, seller]) expect(await withdrawCredits(u.page)).toBe(1);
  }
  // 真实 indexer：anvil 每秒出一块，finalized = 最新块往前 64 块，约一分钟后订单页显示最终确认
  // （fork 链上 anvil_mine 每块要一两秒，比等出块还慢，所以不手动挖块）
  if (h.fakeOrders) h.fake.finalize(orderId);
  await expect(buyer.page.getByTestId("order-level")).toHaveText("已最终确认", {
    timeout: 150_000,
  });
  // 最终确认的同一事务里买卖双方各得 1 次
  if (h.realSponsor) {
    for (const u of [buyer, seller]) expect(await withdrawCredits(u.page)).toBe(2);
  }

  // 默认参数：手续费 max(20 × 1%, 1) = 1 USDT + 基础 gas 费 0.1 USDT，均由卖家承担
  expect(await changes()).toEqual({
    buyer: -U("20.05"),
    seller: U("18.9"),
    escrow: 0n,
    paymaster: U("0.05"),
    feeRecipient: U("1.1"),
  });
  const order = await publicClient.readContract({
    address: chain.contracts.Escrow,
    abi: escrowAbi,
    functionName: "getOrder",
    args: [await chainOrderId(buyer.page, orderId)],
  });
  expect(order.state).toBe(3); // Released
  expect(order.seller).toBe(sellerAddr);
});

test("未登录时引导去论坛登录，登录后回到钱包当前页面", async ({ browser }) => {
  const paths = await backendPaths();
  test.skip(!paths.has("/api/v1/wallet"), "forum-api 没有运行");
  const page = await (await browser.newContext()).newPage();
  await page.goto("/setup");
  await expect(page.getByText("请先登录论坛")).toBeVisible();
  const link = page.getByRole("link", { name: "去登录" });
  await expect(link).toHaveAttribute("href", /\/login\?next=/);
  const href = new URL((await link.getAttribute("href")) ?? "");
  expect(href.pathname).toBe("/login");
  expect(href.searchParams.get("next")).toBe(page.url());
});

test("还没设置用户名时，开通交易先引导去论坛设置，设置后回到钱包", async ({ browser, baseURL }) => {
  const paths = await backendPaths();
  test.skip(!paths.has("/api/v1/wallet"), "forum-api 没有运行");
  const context = await browser.newContext();
  await signIn(context, baseURL ?? "", "nouser", { username: false });
  const page = await context.newPage();
  await page.goto("/setup");
  await expect(page.getByText("先设置用户名")).toBeVisible();
  const href = new URL(
    (await page.getByRole("link", { name: "去论坛设置用户名" }).getAttribute("href")) ?? "",
  );
  expect(href.pathname).toBe("/settings");
  expect(href.searchParams.get("next")).toBe(page.url());
  await context.close();
});
