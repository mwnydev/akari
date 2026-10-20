import { escrowAbi, escrowConfigAbi, localChain as chain } from "@akari/contracts";
import { formatUsdt } from "@akari/ui";
import { expect, test } from "@playwright/test";
import { concat, keccak256, pad, stringToHex, toHex, type Hex } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import type { ApiAuctionAuth, AuctionView } from "../lib/wallet/api";
import { parseAuctionAuth } from "../lib/wallet/auction";
import { FORUM_ORIGIN } from "../lib/wallet/domains";
import { chainIsDown, publicClient } from "./env";
import { seedSettledAuction } from "./forum-user";
import { U, balances, confirmSignature, createWallet, harness, mintUsdt } from "./harness";

// 拍卖：卖家在钱包生成并核对拍卖授权、签名（ERC-6492，钱包未部署）→ 平台签成交证明 → 赢家一次 Face ID 付款建单。
// 拍卖接口由 ./fake-backend.ts 按 forum-api 的形状回答（真实的挂拍、出价、成交证明见 forum-api 的 auction_e2e.rs）。
// 前置：deploy/dev/aa.sh up（部署时已把 EscrowConfig.auctionSigner 设为开发用的 auction-signer）。

const DAY = 86_400;
const AUCTION_ID = 77;
const TOPIC_ID = 5;

/** aa.sh 的 auction-signer（公开派生的开发密钥，与 forum-sponsor 的 SPONSOR_DEV_AUCTION_KEY 相同）。 */
const platform = privateKeyToAccount(keccak256(stringToHex("akari-dev-only:auction-signer")));

test("拍卖：卖家核对并签授权 → 余额不足只给充值地址 → 赢家一次 Face ID 付款建单", async ({
  browser,
  baseURL,
}) => {
  const down = await chainIsDown();
  test.skip(down !== null, down ?? "");
  const h = await harness();
  const base = baseURL ?? "";
  const seller = await h.newUser(browser, "aseller", base);
  const buyer = await h.newUser(browser, "abuyer", base);
  const sellerAddr = await createWallet(seller);
  const buyerAddr = await createWallet(buyer);

  const params = await publicClient.readContract({
    address: chain.contracts.EscrowConfig,
    abi: escrowConfigAbi,
    functionName: "params",
  });
  // 人民币计价，汇率 7.2，保留价 ¥800，手续费买家承担
  const auth: ApiAuctionAuth = {
    seller: sellerAddr,
    token: chain.contracts.MockUSDT,
    auctionId: String(AUCTION_ID),
    reservePrice: "80000",
    currency: 1,
    cnyPerUsdt: 72_000,
    feePayer: 1,
    maxFeeBps: params.feeBps,
    maxBaseGas: params.baseGas.toString(),
    maxMinFee: params.minFee.toString(),
    instant: false,
    shipWindow: 3 * DAY,
    confirmWindow: 7 * DAY,
    listingHash: keccak256(stringToHex(`e2e auction ${Date.now()}`)),
    expiry: Math.floor(Date.now() / 1000) + 30 * DAY,
  };
  // 卖家在论坛填的拍卖草稿（后端 AuctionView，卖家本人看得到保留价）
  const draft = {
    id: AUCTION_ID,
    topic_id: null,
    forum_id: 1,
    seller_id: 1,
    state: "draft",
    currency: "cny",
    cny_per_usdt: 72_000,
    start_price: "50000",
    increment: "1000",
    reserve_price: auth.reservePrice,
    reserve_met: false,
    current_price: "50000",
    current_usdt: "69444445",
    min_next_bid: "50000",
    bid_count: 0,
    eligibility: { kind: "none" },
    terms: { instant: false, ship_window: 3 * DAY, confirm_window: 7 * DAY, fee_payer: 1 },
    duration_secs: 3 * DAY,
    server_time: new Date().toISOString(),
    round: 0,
  } satisfies AuctionView;
  h.fake.addAuction(draft, auth, TOPIC_ID);

  // 卖家：生成授权后确认页逐项显示将要签名的条款，签完回到论坛上线的拍卖帖
  const sp = seller.page;
  await sp.route(`${FORUM_ORIGIN}/**`, (route) =>
    route.fulfill({ contentType: "text/html", body: "<p>forum</p>" }),
  );
  await sp.goto(`/auction/sign?auction=${AUCTION_ID}`);
  await sp.getByRole("button", { name: "生成并核对拍卖授权" }).click();
  const terms = sp.getByTestId("auction-terms");
  await expect(terms).toContainText(`#${AUCTION_ID}`);
  await expect(terms).toContainText("人民币（成交后按下方汇率换算 USDT 结算）");
  await expect(terms).toContainText("1 USDT = 7.2000 元");
  await expect(terms).toContainText("¥800.00");
  await expect(terms).toContainText("折合 111.111112 USDT");
  await expect(terms).toContainText("3 天 / 7 天");
  await expect(terms).toContainText("买家承担");
  await expect(terms).toContainText(`${(params.feeBps / 100).toFixed(2)}%`);
  await expect(terms).toContainText(`${sellerAddr.slice(0, 8)}…${sellerAddr.slice(-6)}`);
  await expect(sp.getByRole("region", { name: "拍卖条件" })).toContainText("¥500.00");

  const signsBefore = await seller.signCount();
  await sp.getByRole("button", { name: "签署拍卖授权" }).click();
  await confirmSignature(sp, "签署拍卖授权", "0.00");
  await expect(sp).toHaveURL(`${FORUM_ORIGIN}/t/${TOPIC_ID}`);
  expect(await seller.signCount()).toBe(signsBefore + 1);

  // 签名链上可验（卖家钱包还没部署：ERC-6492）
  const sellerSig = h.fake.auctionSellerSig(AUCTION_ID) as Hex;
  const parsed = parseAuctionAuth(auth);
  if (!parsed) throw new Error("fixture auth invalid");
  const authHash = await publicClient.readContract({
    address: chain.contracts.Escrow,
    abi: escrowAbi,
    functionName: "hashAuction",
    args: [parsed],
  });
  expect(await publicClient.getCode({ address: sellerAddr })).toBeUndefined();
  expect(
    await publicClient.verifyHash({ address: sellerAddr, hash: authHash, signature: sellerSig }),
  ).toBe(true);

  // 结束：成交价 ¥1,000 → 按卖家汇率向上取整 138.888889 USDT；平台（aa.sh 部署时登记的签名者）签成交证明
  expect(
    await publicClient.readContract({
      address: chain.contracts.EscrowConfig,
      abi: escrowConfigAbi,
      functionName: "auctionSigner",
    }),
    "aa.sh 部署时没有登记 auction-signer，重新运行 aa.sh up",
  ).toBe(platform.address);
  const price = 100_000n;
  const amount = 138_888_889n;
  const payDeadline = Math.floor(Date.now() / 1000) + 2 * DAY;
  const proof = { authHash, winner: buyerAddr, price, payDeadline: BigInt(payDeadline) };
  const proofHash = await publicClient.readContract({
    address: chain.contracts.Escrow,
    abi: escrowAbi,
    functionName: "hashProof",
    args: [proof],
  });
  const platformSig = await platform.sign({ hash: proofHash });
  // 真实 forum-sponsor 只代付库里那份成交证明：把它写进测试库（没有真实后端时由 DEV 代付，不查库）
  if (h.realSponsor) {
    seedSettledAuction({
      chainId: chain.chainId,
      escrow: chain.contracts.Escrow,
      seller: sellerAddr,
      authHash,
      winner: buyerAddr,
      price,
      amount,
      payDeadline,
      signature: platformSig,
    });
  }
  h.fake.settleAuction(AUCTION_ID, {
    auction_id: AUCTION_ID,
    round: 1,
    chain_id: chain.chainId,
    escrow: chain.contracts.Escrow,
    proof: { authHash, winner: buyerAddr, price: price.toString(), payDeadline },
    platform_signature: platformSig,
    currency: "cny",
    cny_per_usdt: 72_000,
    price: price.toString(),
    amount: amount.toString(),
    // 前端以链上参数为准，这几项只是展示用的估算
    buyer_pays: amount.toString(),
    buyer_fee: "0",
    buyer_gas: "0",
    buyer_wallet: buyerAddr,
    pay_deadline: new Date(payDeadline * 1000).toISOString(),
  });

  // 赢家：余额为 0 时只显示充值地址，不给付款按钮
  const bp = buyer.page;
  await bp.goto(`/auction/pay?auction=${AUCTION_ID}`);
  await expect(bp.getByTestId("auction-settlement")).toHaveText(
    "成交价 ¥1,000.00 · 卖家汇率 7.2000 · 折合 138.888889 USDT",
  );
  await expect(bp.getByRole("alert").filter({ hasText: "余额不足" })).toBeVisible();
  await expect(bp.getByTestId("deposit-qr")).toBeVisible();
  await expect(bp.getByRole("button", { name: /^付款/ })).toHaveCount(0);

  // 充值后：费用按链上参数（拍卖范围）算，确认页金额就是合约实际扣的
  await mintUsdt(buyerAddr, U("200"));
  await bp.reload();
  const q = await publicClient.readContract({
    address: chain.contracts.EscrowConfig,
    abi: escrowConfigAbi,
    functionName: "preview",
    args: [2],
  });
  const byRate = (amount * BigInt(q.feeBps)) / 10_000n;
  const fee = q.campaign || byRate > q.minFee ? byRate : q.minFee;
  const total = amount + fee + q.baseGas + 50_000n;
  await expect(bp.getByTestId("pay-total")).toHaveText(`${formatUsdt(total)} USDT`);

  const before = await balances({ buyer: buyerAddr, escrow: chain.contracts.Escrow });
  const signs = await buyer.signCount();
  await bp.getByRole("button", { name: /^付款/ }).click();
  await confirmSignature(bp, "拍卖成交付款", formatUsdt(total));
  await expect(bp.getByTestId("order-link")).toBeVisible({ timeout: 60_000 });
  expect(await buyer.signCount()).toBe(signs + 1);
  const after = await balances({ buyer: buyerAddr, escrow: chain.contracts.Escrow });
  expect(before.buyer - after.buyer).toBe(total);
  expect(after.escrow - before.escrow).toBe(amount + fee + q.baseGas);
  expect(
    await publicClient.readContract({
      address: chain.contracts.Escrow,
      abi: escrowAbi,
      functionName: "auctionSettled",
      args: [keccak256(concat([pad(sellerAddr), toHex(BigInt(AUCTION_ID), { size: 32 })]))],
    }),
  ).toBe(true);
});
