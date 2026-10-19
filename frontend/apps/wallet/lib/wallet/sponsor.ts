import type { Address, Hex } from "viem";
import type { UserOperation } from "viem/account-abstraction";
import { DEV_SPONSOR, chain } from "./config";

/** 需要平台代付的操作。费用档位（ADR 0004）：付款前的操作收固定代付费，付款后的操作免费（结算时扣固定 gas 费）。 */
export type SponsoredAction = "createAndFund" | "markDelivered" | "release";

/** 代付签名覆盖的 UserOperation 字段（除两个签名外的全部字段，见 FixedFeePaymaster.getHash）。 */
export type SponsorableUserOperation = Pick<
  UserOperation<"0.7">,
  | "sender"
  | "nonce"
  | "callData"
  | "callGasLimit"
  | "verificationGasLimit"
  | "preVerificationGas"
  | "maxFeePerGas"
  | "maxPriorityFeePerGas"
> & { factory?: Address | undefined; factoryData?: Hex | undefined };

export interface SponsorRequest {
  action: SponsoredAction;
  userOperation: SponsorableUserOperation;
}

/** 填进 UserOperation 的 Paymaster 字段；`paymasterData` 布局见 FixedFeePaymaster.sol。 */
export interface PaymasterFields {
  paymaster: Address;
  paymasterData: Hex;
  paymasterVerificationGasLimit: bigint;
  paymasterPostOpGasLimit: bigint;
}

/**
 * 代付签名服务。生产实现调用 forum-sponsor（后端校验白名单方法、限频、模拟后签名）；本地链用 ./dev-sponsor.ts。
 * 钱包不信任它：签名前会核对 `paymasterData` 里的费用与确认页上展示的一致（`assertSponsorFee`）。
 */
export interface SponsorClient {
  /** 这次操作要收的代付费（USDT 最小单位），确认页展示用 */
  fee(action: SponsoredAction): Promise<bigint>;
  /** 估算 gas 用的占位字段（签名无效，费用与正式签名相同） */
  stub(action: SponsoredAction): Promise<PaymasterFields>;
  /** gas 字段定稿后的正式代付签名 */
  sponsor(req: SponsorRequest): Promise<PaymasterFields>;
}

/** FixedFeePaymaster.MAX_FEE：单笔代付费硬上限 1 USDT（合约、forum-sponsor、钱包三处一致）。 */
export const MAX_SPONSOR_FEE = 1_000_000n;

/** 从 `paymasterData`（52 字节前缀之后的部分）读出费用：[0:6] validUntil | [6:12] validAfter | [12:28] fee | 签名。 */
export function paymasterDataFee(paymasterData: Hex): bigint {
  if (paymasterData.length < 2 + 28 * 2) throw new Error("代付数据格式不对");
  return BigInt(`0x${paymasterData.slice(2 + 12 * 2, 2 + 28 * 2)}`);
}

/** 代付服务返回的费用必须与用户在确认页看到的完全一致，且不超过硬上限，否则不签名。 */
export function assertSponsorFee(fields: PaymasterFields, expectedFee: bigint): void {
  if (fields.paymaster.toLowerCase() !== chain.contracts.FixedFeePaymaster.toLowerCase()) {
    throw new Error("代付合约地址与配置不符，已停止签名");
  }
  const fee = paymasterDataFee(fields.paymasterData);
  if (fee !== expectedFee || fee > MAX_SPONSOR_FEE) {
    throw new Error("代付费与确认页不一致，已停止签名");
  }
}

export async function getSponsorClient(): Promise<SponsorClient> {
  if (DEV_SPONSOR) {
    // "akari-dev-sponsor" 由 next.config.ts 按环境解析：只有 `next dev` + NEXT_PUBLIC_DEV_SPONSOR=1 才指向真实实现，
    // 其他情况指向不含私钥的 dev-sponsor.disabled.ts；构建后 scripts/assert-no-dev-sponsor.mjs 再查一遍产物。
    const { createDevSponsorClient } = await import("akari-dev-sponsor");
    return createDevSponsorClient();
  }
  throw new Error("代付服务还没有接入（forum-sponsor 上线后启用）");
}
