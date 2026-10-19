import { localChain as chain, mockUsdtAbi } from "@akari/contracts";
import { expect, type Browser, type CDPSession, type Page } from "@playwright/test";
import {
  createWalletClient,
  encodeFunctionData,
  encodePacked,
  http,
  parseAbi,
  parseUnits,
  toHex,
  zeroHash,
  type Address,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { AA_RPC_PATH, backendPaths, publicClient } from "./env";
import { FAKE_EMAIL_CODE, FakeBackend } from "./fake-backend";
import { signIn, waitForCode } from "./forum-user";

// 各端到端测试共用：浏览器用户（CDP 虚拟 Passkey + 真实或模拟的后端）、开通钱包、链上余额与测试资金。

/** deploy/dev/aa.sh 的部署账户（公开的开发密钥）：铸测试 USDT、以 EOA 身份建测试订单。 */
export const DEV_DEPLOYER_KEY =
  "0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81";

const usdt = chain.contracts.MockUSDT;
export const U = (v: string) => parseUnits(v, 6);

export async function balances<K extends string>(
  of: Record<K, Address>,
): Promise<Record<K, bigint>> {
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

export interface User {
  page: Page;
  /** 这个用户的 Passkey 一共签过几次（虚拟认证器的 signCount）：一次 Face ID = +1 */
  signCount(): Promise<number>;
  /** 开通交易第 1 步的邮箱验证码 */
  emailCode(): Promise<string>;
}

export interface Harness {
  fake: FakeBackend;
  /** 订单接口由模拟 indexer 提供（可以手动推进到 finalized） */
  fakeOrders: boolean;
  /** 真实 forum-api 能发交易帖：卖家为交易帖签公开报价，否则在私信会话里签给指定买家 */
  realListings: boolean;
  /** 真实 forum-sponsor（经 forum-api 的 /aa/rpc）：代付规则与提币次数由后端判定 */
  realSponsor: boolean;
  newUser(browser: Browser, tag: string, baseURL: string): Promise<User>;
}

export async function harness(): Promise<Harness> {
  const real = [...(await backendPaths())].map(templateRe);
  const isReal = (path: string) => real.some((re) => re.test(path));
  const fake = new FakeBackend();
  return {
    fake,
    fakeOrders: !isReal("/api/v1/orders/1"),
    realListings: isReal("/api/v1/forums/e2e/listings"),
    realSponsor: isReal(AA_RPC_PATH),
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
export async function createWallet({ page, emailCode }: User): Promise<Address> {
  await page.goto("/setup");
  await page.getByRole("button", { name: "发送验证码到邮箱" }).click();
  await page.getByLabel("邮箱验证码").fill(await emailCode());
  await page.getByRole("button", { name: "验证", exact: true }).click();
  await page.getByRole("button", { name: "创建钱包 Passkey" }).click();
  const text = (await page.getByTestId("wallet-address").textContent()) ?? "";
  expect(text).toMatch(/^0x[0-9a-fA-F]{40}$/);
  return text as Address;
}

/** 部署账户的钱包客户端（本地链，公开的开发密钥）。 */
export const deployer = createWalletClient({
  account: privateKeyToAccount(DEV_DEPLOYER_KEY),
  transport: http(chain.rpcUrl),
});

/** 给地址铸测试 USDT 并等上链。 */
export async function mintUsdt(to: Address, amount: bigint) {
  const hash = await deployer.writeContract({
    chain: null,
    address: usdt,
    abi: mockUsdtAbi,
    functionName: "mint",
    args: [to, amount],
  });
  await publicClient.waitForTransactionReceipt({ hash });
}

/** 确认页必须先出现（动作、金额、网络、地址前后 6 位），再点 Face ID。 */
export async function confirmSignature(
  page: Page,
  action: string,
  amount: string,
  to: string = chain.contracts.Escrow,
) {
  const dialog = page.getByRole("dialog", { name: `确认${action}` });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText(amount);
  await expect(dialog).toContainText("本地测试链");
  await expect(dialog).toContainText(`${to.slice(0, 8)}…${to.slice(-6)}`);
  await dialog.getByRole("button", { name: "使用 Face ID / 指纹确认" }).click();
  await expect(dialog).toBeHidden({ timeout: 120_000 });
}

/** 剩余免费代付提币次数（钱包余额接口，按页面上的登录用户）。 */
export async function withdrawCredits(page: Page): Promise<number> {
  return page.evaluate(async () => {
    const res = await fetch("/api/v1/wallet/balances");
    return ((await res.json()) as { withdraw_credits: number }).withdraw_credits;
  });
}

const kernelExecute = parseAbi(["function execute(bytes32 mode, bytes executionCalldata)"]);

/**
 * 直接向代付服务要一笔「提回交易所」的占位代付（`pm_getPaymasterStubData`，不签名、不上链），
 * 返回 JSON-RPC 错误里的错误码，通过时返回 null。用来确认是后端在拒绝，而不只是页面按钮禁用。
 */
export async function stubWithdraw(
  page: Page,
  wallet: Address,
  to: Address,
): Promise<string | null> {
  const transfer = encodeFunctionData({
    abi: mockUsdtAbi,
    functionName: "transfer",
    args: [to, 1n],
  });
  const callData = encodeFunctionData({
    abi: kernelExecute,
    functionName: "execute",
    args: [zeroHash, encodePacked(["address", "uint256", "bytes"], [usdt, 0n, transfer])],
  });
  const body = JSON.stringify({
    jsonrpc: "2.0",
    id: 1,
    method: "pm_getPaymasterStubData",
    params: [
      { sender: wallet, nonce: toHex(1n), callData },
      chain.entryPoint,
      toHex(chain.chainId),
      {},
    ],
  });
  return page.evaluate(
    async ([path, body]) => {
      const res = await fetch(path, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body,
      });
      const json = (await res.json()) as { error?: { data?: { code?: string } } };
      return json.error ? (json.error.data?.code ?? "UNKNOWN") : null;
    },
    [AA_RPC_PATH, body] as const,
  );
}
