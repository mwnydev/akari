import { getAddress, isAddress, isAddressEqual, zeroAddress, type Address } from "viem";
import type { WithdrawAddressView } from "./api";

// 提回交易所与转账地址的规则（纯函数，不依赖钱包 SDK，可以进首屏 JS；紧急取回页也用）。金额一律 bigint 最小单位。

/** 「全部提回」：余额扣掉代付费后能转出的最多金额（plan 第 5 节：代付费从提出金额中扣）；不够付代付费为 0。 */
export function maxWithdraw(balance: bigint, fee: bigint): bigint {
  return balance > fee ? balance - fee : 0n;
}

/** 提回金额的问题（给用户看的一句话），没问题返回 null。`amount` 为 null 表示输入格式不对。 */
export function withdrawError(amount: bigint | null, balance: bigint, fee: bigint): string | null {
  if (amount === null) return "金额格式不对，最多 6 位小数";
  if (amount <= 0n) return "请输入金额";
  if (amount + fee > balance) {
    return fee > 0n ? "余额不足（还要留出代付费）" : "余额不足";
  }
  return null;
}

/** 免费代付提币次数用完时的提示（界面与 forum-sponsor 的 WITHDRAW_CREDIT_REQUIRED 拒绝共用）。 */
export const NO_WITHDRAW_CREDIT =
  "完成一笔担保交易后可再获得 1 次；也可在「紧急取回」页自付 ETH 提取";

/** 还能不能免费代付提币：剩余次数与今日次数都要 > 0。 */
export function canSponsorWithdraw(credits: number, leftToday: number): boolean {
  return credits > 0 && leftToday > 0;
}

export type AddressCheck = { ok: true; address: Address } | { ok: false; error: string };

/**
 * 校验用户粘贴的收款地址。全小写 / 全大写的地址没有校验和，照收；大小写混排的必须通过 EIP-55 校验和（抄错一位就能发现）。
 * 不能是零地址、自己的钱包或本站合约（USDT、担保合约等转进去就取不回来）。
 */
export function checkAddress(
  input: string,
  { self, contracts = [] }: { self?: string | undefined; contracts?: readonly string[] } = {},
): AddressCheck {
  const raw = input.trim();
  if (!/^0x[0-9a-fA-F]{40}$/.test(raw)) {
    return { ok: false, error: "地址格式不对：应为 0x 开头、共 42 位的 EVM 地址" };
  }
  const body = raw.slice(2);
  const mixedCase = body !== body.toLowerCase() && body !== body.toUpperCase();
  if (mixedCase && !isAddress(raw, { strict: true })) {
    return { ok: false, error: "地址校验和不对，可能抄错了，请从交易所重新复制" };
  }
  const address = getAddress(raw.toLowerCase());
  if (isAddressEqual(address, zeroAddress)) return { ok: false, error: "不能转到零地址" };
  if (self && isAddressEqual(address, getAddress(self))) {
    return { ok: false, error: "这是你自己的钱包地址" };
  }
  if (contracts.some((c) => isAddressEqual(address, getAddress(c)))) {
    return { ok: false, error: "这是合约地址，转进去取不回来，请填交易所的充值地址" };
  }
  return { ok: true, address };
}

/** 提币地址标签：1–20 个字。 */
export function labelError(label: string): string | null {
  const n = [...label.trim()].length;
  return n === 0 ? "请填写备注，如「币安」" : n > 20 ? "备注最多 20 个字" : null;
}

/** 提币地址现在能不能用：没撤销且已过冷却期（倒计时走完就可用，不等重新读取）。 */
export function addressUsable(a: WithdrawAddressView, now: number): boolean {
  return a.status !== "revoked" && Date.parse(a.active_at) <= now;
}
