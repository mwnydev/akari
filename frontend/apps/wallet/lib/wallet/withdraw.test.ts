import { DEFAULT_PARAMS } from "@akari/fees";
import { parseUsdt } from "@akari/ui";
import { describe, expect, it } from "vitest";
import type { WithdrawAddressView } from "./api";
import { chain } from "./config";
import { sponsorFeeFor } from "./sponsor-fee";
import {
  addressUsable,
  canSponsorWithdraw,
  checkAddress,
  labelError,
  maxWithdraw,
  withdrawError,
} from "./withdraw";

const FEE = sponsorFeeFor("transfer");
const SELF = "0x3f9a6c21d84b07e5a1c2d3e4f5a6b7c8d9e0b1c2";
/** EIP-55 例子（eips.ethereum.org/EIPS/eip-55） */
const CHECKSUMMED = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";

describe("提回金额", () => {
  it("提回交易所收代付费，与 plan 默认值一致", () => {
    expect(FEE).toBe(DEFAULT_PARAMS.sponsorFee);
    expect(sponsorFeeFor("claim")).toBe(0n);
    expect(sponsorFeeFor("start")).toBe(0n);
  });

  it("「全部提回」= 余额 − 代付费，按最小单位精确计算", () => {
    expect(maxWithdraw(30_000_000n, FEE)).toBe(29_950_000n);
    expect(maxWithdraw(123_456_789n, FEE)).toBe(123_406_789n);
    // 全部提回后正好扣光，不剩零头
    const all = maxWithdraw(52_340_001n, FEE);
    expect(all + FEE).toBe(52_340_001n);
    expect(withdrawError(all, 52_340_001n, FEE)).toBeNull();
  });

  it("余额不够付代付费时不能提", () => {
    expect(maxWithdraw(FEE, FEE)).toBe(0n);
    expect(maxWithdraw(0n, FEE)).toBe(0n);
    expect(maxWithdraw(FEE + 1n, FEE)).toBe(1n);
  });

  it("金额 + 代付费不能超过余额；格式不对、0 都报错", () => {
    const balance = 10_000_000n;
    expect(withdrawError(parseUsdt("9.95"), balance, FEE)).toBeNull();
    expect(withdrawError(parseUsdt("9.950001"), balance, FEE)).toContain("代付费");
    expect(withdrawError(parseUsdt("10"), balance, 0n)).toBeNull();
    expect(withdrawError(parseUsdt("10.000001"), balance, 0n)).toBe("余额不足");
    expect(withdrawError(parseUsdt("0"), balance, FEE)).toBe("请输入金额");
    expect(withdrawError(parseUsdt("1.0000001"), balance, FEE)).toContain("格式");
  });

  it("免费代付提币：剩余次数与今日次数都要有", () => {
    expect(canSponsorWithdraw(1, 3)).toBe(true);
    expect(canSponsorWithdraw(0, 3)).toBe(false);
    expect(canSponsorWithdraw(2, 0)).toBe(false);
  });
});

describe("收款地址校验", () => {
  it("全小写 / 全大写没有校验和，照收并规范成校验和地址", () => {
    const lower = checkAddress(`  ${CHECKSUMMED.toLowerCase()} `);
    expect(lower).toEqual({ ok: true, address: CHECKSUMMED });
    expect(checkAddress(`0x${CHECKSUMMED.slice(2).toUpperCase()}`)).toEqual({
      ok: true,
      address: CHECKSUMMED,
    });
  });

  it("大小写混排必须通过 EIP-55 校验和", () => {
    expect(checkAddress(CHECKSUMMED).ok).toBe(true);
    const typo = CHECKSUMMED.replace("aAeb", "aaeb");
    expect(checkAddress(typo)).toEqual({ ok: false, error: expect.stringContaining("校验和") });
  });

  it("格式、零地址、自己、本站合约都拒绝", () => {
    expect(checkAddress("TQn9Y2khEsLJW1ChVWFMSMeRDow5KcbLSE").ok).toBe(false); // TRC20 地址
    expect(checkAddress("0x1234").ok).toBe(false);
    expect(checkAddress(`0x${"0".repeat(40)}`).ok).toBe(false);
    expect(checkAddress(SELF.toUpperCase().replace("0X", "0x"), { self: SELF })).toEqual({
      ok: false,
      error: "这是你自己的钱包地址",
    });
    const contracts = Object.values(chain.contracts);
    expect(checkAddress(chain.contracts.MockUSDT.toLowerCase(), { contracts }).ok).toBe(false);
    expect(checkAddress(chain.contracts.Escrow, { contracts }).ok).toBe(false);
  });

  it("备注 1–20 个字", () => {
    expect(labelError("币安")).toBeNull();
    expect(labelError("  ")).not.toBeNull();
    expect(labelError("交".repeat(21))).toContain("20");
  });
});

describe("提币地址冷却", () => {
  const NOW = Date.parse("2026-10-19T12:00:00Z");
  const addr = (o: Partial<WithdrawAddressView>): WithdrawAddressView => ({
    id: 1,
    label: "币安",
    address: CHECKSUMMED,
    status: "active",
    active_at: "2026-10-19T00:00:00Z",
    created_at: "2026-10-19T00:00:00Z",
    ...o,
  });

  it("过了生效时间就能用，倒计时走完不等重新读取", () => {
    expect(addressUsable(addr({}), NOW)).toBe(true);
    expect(addressUsable(addr({ status: "pending", active_at: "2026-10-20T00:00:00Z" }), NOW)).toBe(
      false,
    );
    expect(addressUsable(addr({ status: "pending", active_at: "2026-10-19T11:59:59Z" }), NOW)).toBe(
      true,
    );
  });

  it("撤销的地址不能用", () => {
    expect(addressUsable(addr({ status: "revoked" }), NOW)).toBe(false);
  });
});
