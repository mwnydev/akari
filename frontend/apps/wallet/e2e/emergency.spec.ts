import {
  escrowAbi,
  escrowConfigAbi,
  localChain as chain,
  mockUsdtAbi,
  streamEscrowAbi,
} from "@akari/contracts";
import { expect, test, type Page } from "@playwright/test";
import { keccak256, parseEther, parseEventLogs, stringToHex, type Address, type Hex } from "viem";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { chainIsDown, publicClient } from "./env";
import {
  U,
  balances,
  confirmSignature,
  createWallet,
  deployer,
  harness,
  mintUsdt,
} from "./harness";

// 紧急取回页（ADR 0007）：论坛后端完全不可用（页面上所有 /api/v1 请求都被掐断）时，
// 用钱包 Passkey 两次签名找回钱包，经本地 Alto（不带 Paymaster）提交，gas 用 anvil 打进钱包的 ETH 支付。
// 开通钱包这一步本身还需要后端（真实或模拟），之后的全部操作都不碰后端。

async function anvil(method: string, params: unknown[]) {
  await publicClient.request({ method: method as never, params: params as never });
}

/** 掐断论坛 API，记下页面有没有试图访问它。 */
async function cutBackend(page: Page): Promise<string[]> {
  const hits: string[] = [];
  await page.route("**/api/v1/**", async (route) => {
    hits.push(route.request().url());
    await route.abort();
  });
  return hits;
}

/** 打开紧急取回页 → 连本地节点 → 两次 Face ID 找回钱包，核对地址。 */
async function openEmergency(page: Page, wallet: Address) {
  await page.goto("/emergency");
  await page.getByRole("button", { name: "连接", exact: true }).click();
  await page.getByRole("button", { name: "用 Passkey 找回钱包" }).click();
  await expect(page.getByTestId("emergency-address")).toHaveText(wallet);
}

/** 估算 → 必要时看到「ETH 不够」与充值地址 → 签名提交。 */
async function estimateAndSend(page: Page, action: string, amount: string, to: string) {
  const check = page.getByTestId("gas-check");
  await check.getByRole("button", { name: /估算/ }).click();
  await expect(check.getByTestId("gas-required")).toBeVisible();
  await check.getByRole("button", { name: "签名并提交" }).click();
  await confirmSignature(page, action, amount, to);
  await expect(page.getByTestId("emergency-done")).toBeVisible({ timeout: 120_000 });
}

test("后端停摆：Passkey 找回钱包，自付 ETH 把 USDT 转到任意地址", async ({ browser, baseURL }) => {
  const down = await chainIsDown();
  test.skip(down !== null, down ?? "");
  const h = await harness();
  const user = await h.newUser(browser, "emergency", baseURL ?? "");
  const wallet = await createWallet(user);
  await mintUsdt(wallet, U("12.5"));

  const hits = await cutBackend(user.page);
  const signs = await user.signCount();
  await openEmergency(user.page, wallet);
  expect(await user.signCount()).toBe(signs + 2); // 两次签名算出公钥
  await expect(user.page.getByTestId("emergency-usdt")).toContainText("12.50");

  const to = privateKeyToAccount(generatePrivateKey()).address;
  await user.page.getByLabel(/^收款地址（/).fill(to);
  await user.page.getByRole("button", { name: "全部", exact: true }).click();
  await user.page.getByLabel("再输入一遍收款地址的最后 6 位").fill(to.slice(-6));
  const next = user.page.getByRole("button", { name: "下一步：估算 gas" });
  await expect(next).toBeDisabled(); // 还没勾选网络确认
  await user.page.getByRole("checkbox").check();
  await next.click();

  // 钱包没有 ETH：页面算出要多少，并给出充值地址（就是钱包自己）
  const check = user.page.getByTestId("gas-check");
  await check.getByRole("button", { name: "估算需要的 ETH" }).click();
  await expect(check.getByTestId("fund-address")).toHaveText(wallet);
  await anvil("anvil_setBalance", [wallet, `0x${parseEther("0.5").toString(16)}`]);
  await user.page.getByRole("button", { name: "刷新余额" }).click();
  await expect(user.page.getByTestId("emergency-eth")).toHaveText("0.5");

  await estimateAndSend(user.page, "转出 USDT", "12.50", to);
  expect(await balances({ wallet, to })).toEqual({ wallet: 0n, to: U("12.5") });
  expect(await publicClient.getBalance({ address: wallet })).toBeLessThan(parseEther("0.5"));
  expect(await publicClient.getCode({ address: wallet })).toBeDefined(); // 顺带部署
  expect(hits).toEqual([]);
});

/** 以两个 EOA 建一笔一次性订单和一笔分期订单（部署账户当买家），卖家都不履约。返回链上订单号。 */
async function abandonedOrders(): Promise<{ escrow: bigint; stream: bigint }> {
  const seller = privateKeyToAccount(generatePrivateKey());
  const buyer = deployer.account.address;
  const params = await publicClient.readContract({
    address: chain.contracts.EscrowConfig,
    abi: escrowConfigAbi,
    functionName: "params",
  });
  const expiry = BigInt(Math.floor(Date.now() / 1000) + 3600);
  const common = {
    seller: seller.address,
    buyer,
    token: chain.contracts.MockUSDT,
    amount: U("10"),
    feePayer: 0,
    maxFeeBps: Number(params.feeBps),
    maxBaseGas: params.baseGas,
    maxMinFee: params.minFee,
    bond: 0n,
    lateSlashBps: 0,
    listingHash: keccak256(stringToHex("e2e 紧急取回")),
    expiry,
    maxFills: 1,
  };
  const escrowOffer = {
    ...common,
    instant: false,
    shipWindow: 86_400,
    confirmWindow: 86_400,
    nonce: 1n,
  };
  const streamOffer = {
    ...common,
    maxClaimGas: params.claimGas,
    interval: 1, // 每天
    periods: 5,
    startWindow: 86_400,
    nonce: 2n,
  };
  // EOA 签的就是合约算出的 EIP-712 摘要（hashOffer 是唯一定义）
  const sign = async (hash: Hex) => seller.sign({ hash });
  const escrowSig = await sign(
    await publicClient.readContract({
      address: chain.contracts.Escrow,
      abi: escrowAbi,
      functionName: "hashOffer",
      args: [escrowOffer],
    }),
  );
  const streamSig = await sign(
    await publicClient.readContract({
      address: chain.contracts.StreamEscrow,
      abi: streamEscrowAbi,
      functionName: "hashOffer",
      args: [streamOffer],
    }),
  );
  await mintUsdt(buyer, U("20"));
  const send = async (p: Promise<Hex>) => {
    const r = await publicClient.waitForTransactionReceipt({ hash: await p });
    expect(r.status).toBe("success");
    return r.logs;
  };
  for (const spender of [chain.contracts.Escrow, chain.contracts.StreamEscrow]) {
    await send(
      deployer.writeContract({
        chain: null,
        address: chain.contracts.MockUSDT,
        abi: mockUsdtAbi,
        functionName: "approve",
        args: [spender, U("10")],
      }),
    );
  }
  const escrowLogs = await send(
    deployer.writeContract({
      chain: null,
      address: chain.contracts.Escrow,
      abi: escrowAbi,
      functionName: "createAndFund",
      args: [escrowOffer, escrowSig],
    }),
  );
  const streamLogs = await send(
    deployer.writeContract({
      chain: null,
      address: chain.contracts.StreamEscrow,
      abi: streamEscrowAbi,
      functionName: "createAndFund",
      args: [streamOffer, streamSig],
    }),
  );
  const [e] = parseEventLogs({ abi: escrowAbi, logs: escrowLogs, eventName: "OrderCreated" });
  const [st] = parseEventLogs({
    abi: streamEscrowAbi,
    logs: streamLogs,
    eventName: "OrderCreated",
  });
  if (!e || !st) throw new Error("没有建单事件");
  return { escrow: e.args.orderId, stream: st.args.orderId };
}

test("后端停摆：任何人都能对过期订单触发 claimTimeout，退款给买家", async ({
  browser,
  baseURL,
}) => {
  const down = await chainIsDown();
  test.skip(down !== null, down ?? "");
  const h = await harness();
  const user = await h.newUser(browser, "emergency-timeout", baseURL ?? "");
  const wallet = await createWallet(user);
  await anvil("anvil_setBalance", [wallet, `0x${parseEther("0.5").toString(16)}`]);

  // 链上时间往后拨要撤回，否则之后按真实时间签的代付签名、报价有效期都会失效
  const snapshot = await publicClient.request({ method: "evm_snapshot" as never });
  try {
    const ids = await abandonedOrders();
    await anvil("evm_increaseTime", [86_400 + 60]);
    await anvil("evm_mine", []);

    const hits = await cutBackend(user.page);
    await openEmergency(user.page, wallet);
    await user.page.getByRole("tab", { name: "订单超时处理" }).click();
    for (const [market, label, id, address, abi] of [
      ["escrow", "一次性担保", ids.escrow, chain.contracts.Escrow, escrowAbi],
      ["stream", "分期担保", ids.stream, chain.contracts.StreamEscrow, streamEscrowAbi],
    ] as const) {
      await user.page.getByRole("radio", { name: label }).check();
      await user.page.getByLabel("链上订单号").fill(id.toString());
      await user.page.getByRole("button", { name: /claimTimeout/ }).click();
      await estimateAndSend(user.page, "超时取回 / 放款", "0.00", address);
      const order = await publicClient.readContract({
        address,
        abi,
        functionName: "getOrder",
        args: [id],
      });
      expect(order.state, market).toBe(4); // Refunded
    }
    expect(hits).toEqual([]);
  } finally {
    await anvil("evm_revert", [snapshot]);
  }
});
