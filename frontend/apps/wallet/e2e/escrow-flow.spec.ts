import { escrowAbi, localChain as chain, mockUsdtAbi } from "@akari/contracts";
import { expect, test, type Browser, type CDPSession, type Page } from "@playwright/test";
import { createWalletClient, http, parseUnits, type Address } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { backendPaths, chainIsDown, publicClient } from "./env";
import { FAKE_EMAIL_CODE, FakeBackend } from "./fake-backend";
import { signIn, waitForCode } from "./forum-user";

// 完整担保流程：两个浏览器上下文各带一个 CDP 虚拟 Passkey（卖家、买家），链上余额按前后差额断言。
// 前置：deploy/dev/aa.sh up。链或 bundler 不在线时跳过。
//
// 后端按接口逐个决定：WALLET_E2E_API 上的 forum-api 实现了的接口走真实后端（登录、邮件验证码经 Mailpit），
// 没实现的（或后端没运行）由 ./fake-backend.ts 在浏览器网络层模拟；没有 /aa/rpc 时用 DEV 代付
// （playwright.config.ts 同样据此决定）。全部接口都在时就是完整的真实流程。

/** deploy/dev/aa.sh 的部署账户（公开的开发密钥）；这里只用来给买家铸测试 USDT。 */
const DEV_DEPLOYER_KEY = "0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81";

const usdt = chain.contracts.MockUSDT;
const U = (v: string) => parseUnits(v, 6);

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

/** OpenAPI 路径模板 → 匹配具体路径的正则。 */
const templateRe = (t: string) => new RegExp(`^${t.replace(/\{[^}]+\}/g, "[^/]+")}$`);

interface User {
  page: Page;
  /** 这个用户的 Passkey 一共签过几次（虚拟认证器的 signCount）：一次 Face ID = +1 */
  signCount(): Promise<number>;
  /** 开通交易第 1 步的邮箱验证码 */
  emailCode(): Promise<string>;
}

interface Harness {
  fake: FakeBackend;
  /** 订单接口由模拟 indexer 提供（可以手动推进到 finalized） */
  fakeOrders: boolean;
  newUser(browser: Browser, tag: string, baseURL: string): Promise<User>;
}

async function harness(): Promise<Harness> {
  const real = [...(await backendPaths())].map(templateRe);
  const isReal = (path: string) => real.some((re) => re.test(path));
  const fake = new FakeBackend();
  return {
    fake,
    fakeOrders: !isReal("/api/v1/orders/1"),
    async newUser(browser, tag, baseURL) {
      const context = await browser.newContext();
      // 已登录用户：有真实后端就真的登录，否则模拟后端里每个上下文本身就是一个用户
      const login = isReal("/api/v1/wallet") ? await signIn(context, baseURL, tag) : null;
      await fake.attach(context, (path) => !isReal(path));
      const page = await context.newPage();
      const cdp: CDPSession = await context.newCDPSession(page);
      await cdp.send("WebAuthn.enable");
      // 「已验证用户」的平台认证器：相当于 Face ID 每次都通过
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
        emailCode: async () =>
          login ? (await waitForCode(login.email, login.loginMail)).code : FAKE_EMAIL_CODE,
      };
    },
  };
}

/** 开通交易：邮箱验证码 → 创建 Passkey → 地址登记。 */
async function createWallet({ page, emailCode }: User): Promise<Address> {
  await page.goto("/setup");
  await page.getByRole("button", { name: "发送验证码到邮箱" }).click();
  await page.getByLabel("邮箱验证码").fill(await emailCode());
  await page.getByRole("button", { name: "验证", exact: true }).click();
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

  // 卖家：在私信会话里签一份只给这位买家的 20 USDT 报价（钱包未部署 → ERC-6492 签名，页面用 eth_call 验证）。
  // 注册 Passkey 刚算过一次近期验证，所以这里只有签报价这一次 Face ID。
  const conversation = 1 + Math.floor(Math.random() * 1e9);
  const sellerSigns = await seller.signCount();
  await seller.page.goto(`/offer?conversation=${conversation}`);
  await seller.page.getByLabel("商品 / 服务").fill("e2e 测试商品");
  await seller.page.getByLabel("金额（USDT）").fill("20");
  await seller.page.getByLabel("买家钱包地址").fill(buyerAddr);
  await seller.page.getByRole("button", { name: "签名报价" }).click();
  await confirmSignature(seller.page, "签署担保报价", "20.00");
  await expect(seller.page.getByTestId("offer-verified")).toBeVisible();
  expect(await seller.signCount()).toBe(sellerSigns + 1);
  const link = (await seller.page.getByTestId("offer-link").textContent()) ?? "";
  expect(link).toMatch(/\/pay\?offer=\d+$/);

  // 买家：approve + createAndFund 一个 UserOperation（卖家承担手续费，买家付 20 + 0.05 代付费）
  await buyer.page.goto(link);
  await expect(buyer.page.getByTestId("pay-total")).toContainText("20.05");
  const buyerSigns = await buyer.signCount();
  await buyer.page.getByRole("button", { name: /^付款/ }).click();
  await confirmSignature(buyer.page, "担保付款", "20.05");
  expect(await buyer.signCount()).toBe(buyerSigns + 1); // 授权 + 付款只刷一次 Face ID
  const orderHref = (await buyer.page.getByTestId("order-link").getAttribute("href")) ?? "";
  const orderId = new URL(orderHref, "http://x").searchParams.get("id") ?? "";
  expect(BigInt(orderId)).toBeGreaterThan(0n);

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
  if (h.fakeOrders) {
    await expect(buyer.page.getByTestId("order-level")).toHaveText("已上链（处理中）");
    h.fake.finalize(orderId);
    await expect(buyer.page.getByTestId("order-level")).toHaveText("已最终确认");
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
    args: [BigInt(orderId)],
  });
  expect(order.state).toBe(3); // Released
  expect(order.seller).toBe(sellerAddr);
});

test("未登录时引导去论坛登录", async ({ browser }) => {
  const paths = await backendPaths();
  test.skip(!paths.has("/api/v1/wallet"), "forum-api 没有运行");
  const page = await (await browser.newContext()).newPage();
  await page.goto("/setup");
  await expect(page.getByText("请先登录论坛")).toBeVisible();
  await expect(page.getByRole("link", { name: "去登录" })).toHaveAttribute("href", /\/login$/);
});
