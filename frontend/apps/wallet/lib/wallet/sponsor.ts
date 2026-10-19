import { DEFAULT_PARAMS } from "@akari/fees";
import { http, type Address, type Hex, type Transport } from "viem";
import {
  createPaymasterClient,
  type GetPaymasterDataParameters,
  type GetPaymasterDataReturnType,
  type GetPaymasterStubDataParameters,
} from "viem/account-abstraction";
import { AA_RPC_URL, DEV_SPONSOR, chain } from "./config";

/** 需要平台代付的操作。费用档位（ADR 0004）：付款前的操作收固定代付费，付款后的操作免费（结算时扣固定 gas 费）。 */
export type SponsoredAction = "createAndFund" | "markDelivered" | "release";

/**
 * 确认页展示的代付费，与 forum-sponsor 的规则一致：付款前的操作收费，付款后的操作免费。
 * 站长调价后这里与代付服务不一致时，`assertSponsorFee` 会拒签，不会多扣用户的钱。
 */
export function sponsorFeeFor(action: SponsoredAction): bigint {
  return action === "createAndFund" ? DEFAULT_PARAMS.sponsorFee : 0n;
}

/** 填进 UserOperation 的 Paymaster 字段；`paymasterData` 布局见 FixedFeePaymaster.sol。 */
export interface PaymasterFields {
  paymaster: Address;
  paymasterData: Hex;
  paymasterVerificationGasLimit?: bigint | undefined;
  paymasterPostOpGasLimit: bigint;
}

/**
 * 代付签名服务（ERC-7677 的两个方法）。生产实现走 forum-api 的 `/api/v1/aa/rpc`（ADR 0009，后端校验白名单方法、
 * 限频、模拟后签名）；本地链没有后端代付时用 ./dev-sponsor.ts。
 * 钱包不信任它：签名前核对 `paymasterData` 里的费用与确认页上展示的一致（`assertSponsorFee`）。
 */
export interface SponsorClient {
  /** 估算 gas 用的占位字段（签名无效，费用与正式签名相同） */
  getPaymasterStubData(
    action: SponsoredAction,
    params: GetPaymasterStubDataParameters,
  ): Promise<PaymasterFields>;
  /** gas 字段定稿后的正式代付签名 */
  getPaymasterData(
    action: SponsoredAction,
    params: GetPaymasterDataParameters,
  ): Promise<PaymasterFields>;
}

/** FixedFeePaymaster.MAX_FEE：单笔代付费硬上限 1 USDT（合约、forum-sponsor、钱包三处一致）。 */
export const MAX_SPONSOR_FEE = 1_000_000n;

/** 从 `paymasterData`（52 字节前缀之后的部分）读出费用：[0:6] validUntil | [6:12] validAfter | [12:28] fee | 签名。 */
export function paymasterDataFee(paymasterData: Hex): bigint {
  if (paymasterData.length < 2 + 28 * 2) throw new Error("代付数据格式不对");
  return BigInt(`0x${paymasterData.slice(2 + 12 * 2, 2 + 28 * 2)}`);
}

/**
 * 代付服务返回的 Paymaster 必须是链配置里某个钱包版本的 Paymaster（ADR 0008），
 * 费用必须与用户在确认页看到的完全一致且不超过硬上限，否则不签名。
 */
export function assertSponsorFee(
  fields: PaymasterFields,
  expectedFee: bigint,
  paymasters: readonly string[],
): void {
  if (!paymasters.includes(fields.paymaster.toLowerCase())) {
    throw new Error("代付合约地址与配置不符，已停止签名");
  }
  const fee = paymasterDataFee(fields.paymasterData);
  if (fee !== expectedFee || fee > MAX_SPONSOR_FEE) {
    throw new Error("代付费与确认页不一致，已停止签名");
  }
}

/** bundler 入口：生产走 forum-api（带论坛会话 Cookie）；DEV 代付直连本地 Alto。 */
export function bundlerTransport(): Transport {
  return DEV_SPONSOR
    ? http(chain.bundlerUrl)
    : http(AA_RPC_URL, { fetchOptions: { credentials: "include" } });
}

/** ERC-7677 结果只接受 EntryPoint v0.7 的拆分字段（不是 v0.6 的 `paymasterAndData`）。 */
function v07Fields(r: GetPaymasterDataReturnType): PaymasterFields {
  if (!r.paymaster || !r.paymasterData) throw new Error("代付服务返回的数据格式不对");
  return {
    paymaster: r.paymaster,
    paymasterData: r.paymasterData,
    paymasterVerificationGasLimit: r.paymasterVerificationGasLimit,
    paymasterPostOpGasLimit: r.paymasterPostOpGasLimit ?? 0n, // FixedFeePaymaster 不实现 postOp
  };
}

/** 生产代付：viem 的 ERC-7677 客户端，`context.action` 告诉后端这次操作属于哪一档。 */
function createRpcSponsorClient(): SponsorClient {
  const paymaster = createPaymasterClient({ transport: bundlerTransport() });
  return {
    getPaymasterStubData: async (action, params) =>
      v07Fields(await paymaster.getPaymasterStubData({ ...params, context: { action } })),
    getPaymasterData: async (action, params) =>
      v07Fields(await paymaster.getPaymasterData({ ...params, context: { action } })),
  };
}

export async function getSponsorClient(): Promise<SponsorClient> {
  if (DEV_SPONSOR) {
    // "akari-dev-sponsor" 由 next.config.ts 按环境解析：只有 `next dev` + NEXT_PUBLIC_DEV_SPONSOR=1 才指向真实实现，
    // 其他情况指向不含私钥的 dev-sponsor.disabled.ts；构建后 scripts/assert-no-dev-sponsor.mjs 再查一遍产物。
    const { createDevSponsorClient } = await import("akari-dev-sponsor");
    return createDevSponsorClient();
  }
  return createRpcSponsorClient();
}
