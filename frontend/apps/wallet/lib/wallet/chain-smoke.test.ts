import {
  escrowAbi,
  escrowConfigAbi,
  mockUsdtAbi,
  type ChainConfig,
  type WalletInitVersion,
} from "@akari/contracts";
import { p256 } from "@noble/curves/p256";
import type { WebAuthnKey } from "@zerodev/webauthn-key";
import { createKernelAccountClient } from "@zerodev/sdk";
import { readFileSync, writeFileSync } from "node:fs";
import {
  bytesToHex,
  createPublicClient,
  decodeFunctionData,
  defineChain,
  encodeFunctionData,
  getAddress,
  http,
  isAddressEqual,
  keccak256,
  parseAbi,
  toHex,
  zeroAddress,
  type Hex,
} from "viem";
import { createPaymasterClient } from "viem/account-abstraction";
import { privateKeyToAccount } from "viem/accounts";
import { getEip712Domain } from "viem/actions";
import { describe, expect, it } from "vitest";
import { bundlerGasPrice, kernelAccount } from "./kernel";
import { paymasterDataFee } from "./sponsor";
import { OFFER_TYPES } from "./typed-data";

// 部署后的链上冒烟检查（deploy/testnet/rehearse.sh smoke 设置下面的环境变量；平时跳过）：
//   CHAIN_SMOKE_JSON=<testnet.sh emit-config 写出的前端链配置>  CHAIN_SMOKE_RPC=<该链的节点>
//     钱包 SDK 按最新初始化参数版本推算的地址 = 链上 Kernel 工厂 getAddress 的结果，initCode 里授权的是本版本的 Paymaster；
//     CHAIN_SMOKE_OUT 给出时把推算的地址写进该文件。只读，可以对真实测试网运行。
//   另设 CHAIN_SMOKE_SPONSOR_URL / CHAIN_SMOKE_SPONSOR_TOKEN / CHAIN_SMOKE_USER_ID / CHAIN_SMOKE_SELLER_KEY 时，
//     经 forum-sponsor 模拟并签一笔付款（approve + createAndFund）：钱包要已登记在该用户名下、有足够 USDT。

const env = process.env;
const chainJson = env.CHAIN_SMOKE_JSON ?? "";
const rpc = env.CHAIN_SMOKE_RPC ?? "";
const sponsorUrl = env.CHAIN_SMOKE_SPONSOR_URL ?? "";
const sponsorToken = env.CHAIN_SMOKE_SPONSOR_TOKEN ?? "";
const userId = env.CHAIN_SMOKE_USER_ID ?? "";
const sellerKey = env.CHAIN_SMOKE_SELLER_KEY ?? "";
const ready = chainJson !== "" && rpc !== "";
const sponsorReady = sponsorUrl !== "" && sponsorToken !== "" && userId !== "" && sellerKey !== "";

const factoryAbi = parseAbi([
  "function implementation() view returns (address)",
  "function getAddress(bytes data, bytes32 salt) view returns (address)",
]);
const metaFactoryAbi = parseAbi([
  "function deployWithFactory(address factory, bytes createData, bytes32 salt) payable returns (address)",
]);

/** 固定的测试 P-256 公钥（私钥是公开常量，只用来推算地址；这里不签任何东西）。 */
function smokeKey(): WebAuthnKey {
  const pub = p256.getPublicKey(keccak256(toHex("akari-chain-smoke")).slice(2), false);
  const credential = toHex("akari-chain-smoke-credential");
  return {
    pubX: BigInt(bytesToHex(pub.slice(1, 33))),
    pubY: BigInt(bytesToHex(pub.slice(33, 65))),
    authenticatorId: credential,
    authenticatorIdHash: keccak256(credential),
    rpID: "localhost",
    signMessageCallback: () => Promise.reject(new Error("smoke key never signs")),
  };
}

/** 部署记录生成的前端链配置、连到该链的客户端，以及最新的钱包初始化参数版本。 */
function setup() {
  const cfg = JSON.parse(readFileSync(chainJson, "utf8")) as ChainConfig;
  const client = createPublicClient({
    chain: defineChain({
      id: cfg.chainId,
      name: cfg.name,
      nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
      rpcUrls: { default: { http: [rpc] } },
    }),
    transport: http(rpc),
  });
  const v: WalletInitVersion | undefined = cfg.walletInitVersions.at(-1);
  if (!v) throw new Error("链配置里没有钱包初始化参数版本");
  return { cfg, client, v };
}

describe.skipIf(!ready)("chain smoke（部署后核对）", () => {
  it("钱包地址与链上 Kernel 工厂一致", { timeout: 60_000 }, async () => {
    const { client, v } = setup();
    const account = await kernelAccount(client, smokeKey(), v);
    const { factory, factoryData } = await account.getFactoryArgs();
    if (!factory || !factoryData) throw new Error("测试钱包已经部署过（没有 initCode）");
    expect(isAddressEqual(factory, v.metaFactory)).toBe(true);
    const { args } = decodeFunctionData({ abi: metaFactoryAbi, data: factoryData });
    const [kernelFactory, createData, salt] = args;
    expect(isAddressEqual(kernelFactory, v.factory)).toBe(true);
    const impl = await client.readContract({
      address: v.factory,
      abi: factoryAbi,
      functionName: "implementation",
    });
    expect(isAddressEqual(impl, v.kernelImpl)).toBe(true);
    const onchain = await client.readContract({
      address: v.factory,
      abi: factoryAbi,
      functionName: "getAddress",
      args: [createData, salt],
    });
    expect(getAddress(onchain)).toBe(getAddress(account.address));
    // ADR 0004：initCode 里给本版本的 Paymaster 授权 USDT（代付费在验证阶段扣）
    const approve = encodeFunctionData({
      abi: mockUsdtAbi,
      functionName: "approve",
      args: [v.paymaster, BigInt(v.approveAmount)],
    });
    expect(createData.toLowerCase()).toContain(approve.slice(2).toLowerCase());
    if (env.CHAIN_SMOKE_OUT) writeFileSync(env.CHAIN_SMOKE_OUT, account.address);
  });

  it.skipIf(!sponsorReady)("forum-sponsor 模拟并签一笔付款", { timeout: 120_000 }, async () => {
    const { cfg, client, v } = setup();
    const account = await kernelAccount(client, smokeKey(), v);
    const seller = privateKeyToAccount(sellerKey as Hex);
    const { Escrow: escrow, EscrowConfig: config, MockUSDT: token } = cfg.contracts;
    const params = await client.readContract({
      address: config,
      abi: escrowConfigAbi,
      functionName: "params",
    });
    const now = BigInt(Math.floor(Date.now() / 1000));
    const offer = {
      seller: seller.address,
      buyer: zeroAddress,
      token,
      amount: 10_000_000n,
      feePayer: 0, // 卖家承担手续费与 gas 费：买家只付货款
      maxFeeBps: params.feeBps,
      maxBaseGas: params.baseGas,
      maxMinFee: params.minFee,
      bond: 0n,
      lateSlashBps: 0,
      instant: false,
      shipWindow: 3 * 86_400,
      confirmWindow: 7 * 86_400,
      listingHash: keccak256(toHex("akari-chain-smoke-listing")),
      expiry: Number(now + 3600n),
      maxFills: 1,
      nonce: now,
    } as const;
    const { domain } = await getEip712Domain(client, { address: escrow });
    const { name, version, chainId, verifyingContract } = domain;
    const signature = await seller.signTypedData({
      domain: { name, version, chainId, verifyingContract },
      types: OFFER_TYPES,
      primaryType: "Offer",
      message: offer,
    });

    // 与钱包页面相同的 ERC-7677 + bundler 调用，只是直连 forum-sponsor（生产经 forum-api 转发并带上用户 ID）
    const sponsor = http(`${sponsorUrl}/rpc`, {
      fetchOptions: {
        headers: { authorization: `Bearer ${sponsorToken}`, "x-forum-user-id": userId },
      },
    });
    const paymaster = createPaymasterClient({ transport: sponsor });
    const context = { action: "createAndFund" };
    const kernel = createKernelAccountClient({
      account,
      chain: client.chain,
      client,
      bundlerTransport: sponsor,
      paymaster: {
        getPaymasterStubData: (p) => paymaster.getPaymasterStubData({ ...p, context }),
        getPaymasterData: (p) => paymaster.getPaymasterData({ ...p, context }),
      },
      userOperation: { estimateFeesPerGas: bundlerGasPrice(sponsor) },
    });
    const op = await kernel.prepareUserOperation({
      calls: [
        { to: token, abi: mockUsdtAbi, functionName: "approve", args: [escrow, offer.amount] },
        { to: escrow, abi: escrowAbi, functionName: "createAndFund", args: [offer, signature] },
      ],
    });
    if (!op.paymaster || !op.paymasterData || !op.factory)
      throw new Error("代付结果缺少 Paymaster 或 initCode");
    expect(isAddressEqual(op.paymaster, v.paymaster)).toBe(true);
    expect(paymasterDataFee(op.paymasterData)).toBe(50_000n); // 付款前操作收默认代付费 0.05 USDT
    expect(isAddressEqual(op.factory, v.metaFactory)).toBe(true);
  });
});
