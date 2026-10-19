import type { WalletInitVersion } from "@akari/contracts";
import {
  decodeFunctionData,
  encodeErrorResult,
  keccak256,
  parseAbi,
  slice,
  type Address,
  type Hex,
} from "viem";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NewWallet, PasskeyView, TradeParams, WalletApi, WalletOverview } from "./api";
import { chain } from "./config";
import { toView } from "./init-version";

// ADR 0008：钱包地址由初始化参数版本决定。这里用假的 JSON-RPC 节点代替链：EntryPoint.getSenderAddress
// 返回 initCode 哈希的后 20 字节，所以「地址相同」⇔「initCode 完全相同」。

/** 链配置里再加一个换了 Paymaster 的版本 2（例如 Paymaster 修漏洞后重新部署）。 */
vi.mock("./config", async (importOriginal) => {
  const real = await importOriginal<typeof import("./config")>();
  const [first] = real.chain.walletInitVersions;
  const second = { ...first, version: 2, paymaster: "0x3333333333333333333333333333333333333333" };
  return { ...real, chain: { ...real.chain, walletInitVersions: [first, second] } };
});

const [v1, v2] = chain.walletInitVersions as [WalletInitVersion, WalletInitVersion];

// 开通流程里浏览器创建 Passkey 的那一步：返回什么不重要，模拟后端直接给出登记结果
vi.mock("@simplewebauthn/browser", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@simplewebauthn/browser")>()),
  startRegistration: async () => ({ id: "credential" }),
}));

const entryPointAbi = parseAbi([
  "function getSenderAddress(bytes initCode)",
  "error SenderAddressResult(address sender)",
]);

const senderOf = (initCode: Hex): Address => slice(keccak256(initCode), 12);

function fakeNode(): typeof fetch {
  return async (_url, init) => {
    const { id, method, params } = JSON.parse(String(init?.body)) as {
      id: number;
      method: string;
      params: [{ to: Address; data: Hex }];
    };
    const reply = (body: object) =>
      new Response(JSON.stringify({ jsonrpc: "2.0", id, ...body }), {
        headers: { "content-type": "application/json" },
      });
    if (method === "eth_chainId") return reply({ result: `0x${chain.chainId.toString(16)}` });
    if (method === "eth_call" && params[0].to.toLowerCase() === v1.entryPoint.toLowerCase()) {
      const { args } = decodeFunctionData({ abi: entryPointAbi, data: params[0].data });
      const data = encodeErrorResult({
        abi: entryPointAbi,
        errorName: "SenderAddressResult",
        args: [senderOf(args[0])],
      });
      return reply({ error: { code: 3, message: "execution reverted", data } });
    }
    return reply({ error: { code: -32601, message: `unexpected ${method}` } });
  };
}

const passkey: PasskeyView = {
  id: 7,
  credential_id: "AQIDBAUGBwgJCgsMDQ4PEA",
  pubkey_x: `0x${"11".repeat(32)}`,
  pubkey_y: `0x${"22".repeat(32)}`,
  created_at: "2026-10-19T00:00:00Z",
};

/** 单元测试用的内存后端：只实现开通 / 打开钱包会用到的接口。 */
function memoryApi(state: { overview: WalletOverview; remote: WalletInitVersion[] }) {
  const bound: NewWallet[] = [];
  const unused = () => Promise.reject(new Error("测试里没有模拟这个接口"));
  const api: WalletApi = {
    overview: async () => state.overview,
    tradeParams: async () => ({ wallet_init_versions: state.remote.map(toView) }) as TradeParams,
    registrationOptions: async () => ({}) as never,
    finishRegistration: async () => passkey,
    bindWallet: async (w) => {
      bound.push(w);
      return { ...w, passkey_id: passkey.id, created_at: passkey.created_at };
    },
    startReverify: unused,
    verifyReverify: unused,
    assertionOptions: unused,
    finishAssertion: unused,
    listing: unused,
    createOffer: unused,
    acceptOffer: unused,
    order: unused,
    orders: unused,
    offerOrder: unused,
    balances: unused,
    withdrawAddresses: unused,
    addWithdrawAddress: unused,
    removeWithdrawAddress: unused,
    auction: unused,
    startAuction: unused,
    finishAuction: unused,
    auctionPayment: unused,
  };
  return { api, bound };
}

const overviewWith = (address: Address, init_version = 1): WalletOverview => ({
  enabled: true,
  passkeys: [passkey],
  wallets: [
    {
      chain_id: chain.chainId,
      address,
      passkey_id: passkey.id,
      created_at: passkey.created_at,
      init_version,
    },
  ],
});

describe("钱包初始化参数版本（ADR 0008）", () => {
  beforeEach(() => vi.stubGlobal("fetch", fakeNode()));
  afterEach(() => vi.unstubAllGlobals());

  it("地址取决于版本：换了 Paymaster 的版本算出不同的地址", async () => {
    const { walletAddress } = await import("./sdk");
    const a1 = await walletAddress(passkey, v1);
    expect(a1).toMatch(/^0x[0-9a-fA-F]{40}$/);
    expect(await walletAddress(passkey, v1)).toBe(a1);
    expect(await walletAddress(passkey, v2)).not.toBe(a1);
  });

  it("新增版本后，版本 1 的老钱包仍按版本 1 打开，地址不变", async () => {
    const { openWallet, walletAddress } = await import("./sdk");
    const old = await walletAddress(passkey, v1);
    const { api } = memoryApi({ overview: overviewWith(old, 1), remote: [v1, v2] });
    const wallet = await openWallet(api, await api.overview());
    expect(wallet.address).toBe(old);
    expect(wallet.initVersion).toBe(1);
  });

  it("新开通的钱包用最新版本，并把版本号一起登记", async () => {
    const { createWallet, walletAddress } = await import("./sdk");
    const { api, bound } = memoryApi({ overview: overviewWith(v1.entryPoint), remote: [v1, v2] });
    const wallet = await createWallet(api);
    expect(wallet.initVersion).toBe(2);
    expect(bound).toEqual([
      { chain_id: chain.chainId, address: await walletAddress(passkey, v2), init_version: 2 },
    ]);
  });

  it("后端还不认识最新版本时，新钱包用双方都认识的版本", async () => {
    const { createWallet } = await import("./sdk");
    const { api, bound } = memoryApi({ overview: overviewWith(v1.entryPoint), remote: [v1] });
    await createWallet(api);
    expect(bound[0]?.init_version).toBe(1);
  });

  it("算出的地址与登记的不一致时拒绝打开（不显示可能收不到钱的地址）", async () => {
    const { openWallet, walletAddress } = await import("./sdk");
    const { api } = memoryApi({
      overview: overviewWith(await walletAddress(passkey, v2), 1),
      remote: [v1, v2],
    });
    await expect(openWallet(api, await api.overview())).rejects.toThrow("与登记的不一致");
  });
});
