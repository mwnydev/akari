import { escrowAbi, localChain as chain, mockUsdtAbi } from "@akari/contracts";
import { expect, test, type Browser, type CDPSession, type Page } from "@playwright/test";
import { createPublicClient, createWalletClient, http, parseUnits, type Address } from "viem";
import { privateKeyToAccount } from "viem/accounts";

// 完整担保流程：两个浏览器上下文各带一个 CDP 虚拟 Passkey（卖家、买家），链上余额按前后差额断言。
// 前置：deploy/dev/aa.sh up。链或 bundler 不在线时跳过。

/** deploy/dev/aa.sh 的部署账户（公开的开发密钥）；这里只用来给买家铸测试 USDT。 */
const DEV_DEPLOYER_KEY = "0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81";

const publicClient = createPublicClient({ transport: http(chain.rpcUrl) });
const usdt = chain.contracts.MockUSDT;
const U = (v: string) => parseUnits(v, 6);

async function chainIsUp(): Promise<string | null> {
  try {
    if ((await publicClient.getChainId()) !== chain.chainId)
      return "anvil 的链 ID 与 chain.local.json 不符";
    if (!(await publicClient.getCode({ address: chain.contracts.Escrow }))) {
      return "Escrow 没有部署（chain.local.json 与当前链不符，重新运行 aa.sh up）";
    }
    const res = await fetch(chain.bundlerUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "eth_supportedEntryPoints",
        params: [],
      }),
    });
    if (!res.ok) return "bundler 没有响应";
    return null;
  } catch {
    return "本地 AA 开发链没有启动（deploy/dev/aa.sh up）";
  }
}

async function balances<K extends string>(of: Record<K, Address>): Promise<Record<K, bigint>> {
  const entries = await Promise.all(
    (Object.entries(of) as [K, Address][]).map(async ([name, address]) => {
      const balance = await publicClient.readContract({
        address: usdt,
        abi: mockUsdtAbi,
        functionName: "balanceOf",
        args: [address],
      });
      return [name, balance] as const;
    }),
  );
  return Object.fromEntries(entries) as Record<K, bigint>;
}

interface User {
  page: Page;
  /** 这个用户的 Passkey 一共签过几次（虚拟认证器的 signCount）：一次 Face ID = +1 */
  signCount(): Promise<number>;
}

/** 新开一个浏览器上下文，挂一个「已验证用户」的平台认证器（相当于 Face ID 每次都通过）。 */
async function userWithPasskey(browser: Browser): Promise<User> {
  const context = await browser.newContext();
  const page = await context.newPage();
  const cdp: CDPSession = await context.newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return {
    page,
    async signCount() {
      const { credentials } = await cdp.send("WebAuthn.getCredentials", { authenticatorId });
      const [credential, ...others] = credentials;
      if (!credential || others.length > 0) throw new Error("每个用户应当恰好有一个 Passkey");
      return credential.signCount;
    },
  };
}

async function createWallet(page: Page): Promise<Address> {
  await page.goto("/setup");
  await page.getByRole("button", { name: "创建钱包 Passkey" }).click();
  const text = (await page.getByTestId("wallet-address").textContent()) ?? "";
  expect(text).toMatch(/^0x[0-9a-fA-F]{40}$/);
  return text as Address;
}

/** 确认页必须先出现（动作、金额、网络、地址前后 6 位），再点 Face ID。 */
async function confirmSignature(page: Page, action: string, amount: string) {
  const dialog = page.getByRole("dialog", { name: `确认${action}` });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(amount);
  await expect(dialog).toContainText("本地测试链");
  await expect(dialog).toContainText(
    `${chain.contracts.Escrow.slice(0, 8)}…${chain.contracts.Escrow.slice(-6)}`,
  );
  await dialog.getByRole("button", { name: "使用 Face ID / 指纹确认" }).click();
  await expect(dialog).toBeHidden();
}

test("卖家签报价 → 买家一次 Face ID 付款 → 卖家发货 → 买家确认收货", async ({ browser }) => {
  const down = await chainIsUp();
  test.skip(down !== null, down ?? "");

  const { page: seller } = await userWithPasskey(browser);
  const { page: buyer, signCount: buyerSignCount } = await userWithPasskey(browser);
  const sellerAddr = await createWallet(seller);
  const buyerAddr = await createWallet(buyer);
  expect(sellerAddr).not.toBe(buyerAddr);
  // 只算出地址，还没上链
  expect(await publicClient.getCode({ address: sellerAddr })).toBeUndefined();

  const deployer = privateKeyToAccount(DEV_DEPLOYER_KEY);
  const walletClient = createWalletClient({ account: deployer, transport: http(chain.rpcUrl) });
  const mintTx = await walletClient.writeContract({
    chain: null,
    address: usdt,
    abi: mockUsdtAbi,
    functionName: "mint",
    args: [buyerAddr, U("50")],
  });
  await publicClient.waitForTransactionReceipt({ hash: mintTx });

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

  // 卖家：签一份只给这位买家的 20 USDT 报价（钱包未部署 → ERC-6492 签名，页面用 eth_call 验证）
  await seller.goto("/offer");
  await seller.getByLabel("商品 / 服务").fill("e2e 测试商品");
  await seller.getByLabel("金额（USDT）").fill("20");
  await seller.getByLabel("买家钱包地址").fill(buyerAddr);
  await seller.getByRole("button", { name: "签名报价" }).click();
  await confirmSignature(seller, "签署担保报价", "20.00");
  await expect(seller.getByTestId("offer-verified")).toBeVisible();
  const link = (await seller.getByTestId("offer-link").textContent()) ?? "";

  // 买家：approve + createAndFund 一个 UserOperation（卖家承担手续费，买家付 20 + 0.05 代付费）
  await buyer.goto(link);
  await expect(buyer.getByTestId("pay-total")).toContainText("20.05");
  const signsBefore = await buyerSignCount();
  await buyer.getByRole("button", { name: /^付款/ }).click();
  await confirmSignature(buyer, "担保付款", "20.05");
  expect(await buyerSignCount()).toBe(signsBefore + 1); // 授权 + 付款只刷一次 Face ID
  const orderHref = (await buyer.getByTestId("order-link").getAttribute("href")) ?? "";
  const orderId = BigInt(new URL(orderHref, "http://x").searchParams.get("id") ?? "0");
  expect(orderId).toBeGreaterThan(0n);

  expect(await changes()).toEqual({
    buyer: -U("20.05"),
    seller: 0n,
    escrow: U("20"), // 托管
    paymaster: U("0.05"), // 验证阶段收的代付费
    feeRecipient: 0n,
  });
  expect(await publicClient.getCode({ address: buyerAddr })).toBeDefined(); // 首次操作顺带部署

  // 卖家：标记发货（付款后的操作免代付费；卖家钱包这时才部署）
  await seller.goto(`/order?id=${orderId}`);
  await seller.getByRole("button", { name: "标记已发货" }).click();
  await confirmSignature(seller, "标记已发货", "20.00");
  await expect(seller.getByTestId("order-state")).toHaveText("卖家已发货，等待买家确认收货");

  // 买家：确认收货 → 放款
  await buyer.goto(`/order?id=${orderId}`);
  await buyer.getByRole("button", { name: "确认收货" }).click();
  await confirmSignature(buyer, "确认收货", "20.00");
  await expect(buyer.getByTestId("order-state")).toHaveText("已确认收货，货款已放给卖家");

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
    args: [orderId],
  });
  expect(order.state).toBe(3); // Released
  expect(order.seller).toBe(sellerAddr);
});
