import { encodePacked } from "viem";
import { describe, expect, it } from "vitest";
import { chain } from "./config";
import { assertSponsorFee, paymasterDataFee, type PaymasterFields } from "./sponsor";

const data = (fee: bigint) =>
  encodePacked(
    ["uint48", "uint48", "uint128", "bytes"],
    [1_800_000_120, 1_800_000_000, fee, `0x${"11".repeat(65)}`],
  );

const fields = (fee: bigint, paymaster = chain.contracts.FixedFeePaymaster): PaymasterFields => ({
  paymaster,
  paymasterData: data(fee),
  paymasterVerificationGasLimit: 120_000n,
  paymasterPostOpGasLimit: 0n,
});

describe("代付数据核对", () => {
  it("按 FixedFeePaymaster 布局读出费用", () => {
    expect(paymasterDataFee(data(50_000n))).toBe(50_000n);
    expect(paymasterDataFee(data(0n))).toBe(0n);
  });

  it("数据太短直接报错", () => {
    expect(() => paymasterDataFee("0x1234")).toThrow();
  });

  it("费用与确认页一致才放行", () => {
    expect(() => assertSponsorFee(fields(50_000n), 50_000n)).not.toThrow();
    expect(() => assertSponsorFee(fields(60_000n), 50_000n)).toThrow("代付费与确认页不一致");
  });

  it("超过 1 USDT 硬上限一律拒绝，即使与展示一致", () => {
    expect(() => assertSponsorFee(fields(1_000_001n), 1_000_001n)).toThrow();
  });

  it("拒绝不是配置里的 Paymaster", () => {
    expect(() =>
      assertSponsorFee(fields(0n, "0x3333333333333333333333333333333333333333"), 0n),
    ).toThrow("代付合约地址与配置不符");
  });
});
