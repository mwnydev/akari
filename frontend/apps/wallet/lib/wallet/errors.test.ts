import { escrowAbi } from "@akari/contracts";
import { BaseError, ContractFunctionRevertedError, encodeErrorResult } from "viem";
import { describe, expect, it } from "vitest";
import { humanizeError } from "./errors";

describe("错误翻译", () => {
  it("取消 Face ID", () => {
    const e = Object.assign(new Error("The operation either timed out or was not allowed."), {
      name: "NotAllowedError",
    });
    expect(humanizeError(e).message).toBe("Face ID / 指纹验证已取消或超时，没有签名");
  });

  it("合约自定义错误 → 中文，原始错误保留在 cause", () => {
    const reverted = new ContractFunctionRevertedError({
      abi: escrowAbi,
      functionName: "createAndFund",
      data: encodeErrorResult({ abi: escrowAbi, errorName: "OfferUnavailable" }),
    });
    const wrapped = new BaseError("UserOperation reverted", { cause: reverted });
    const out = humanizeError(wrapped);
    expect(out.message).toBe("报价已过期、已撤销或已被使用");
    expect(out.cause).toBe(wrapped);
  });

  it("Paymaster 拒绝（AA3x）", () => {
    expect(humanizeError(new BaseError("AA34 signature error")).message).toBe(
      "平台代付暂时不可用，请稍后再试",
    );
  });

  it("已经是中文的提示原样返回，其他兜底", () => {
    const zh = new Error("余额不足");
    expect(humanizeError(zh)).toBe(zh);
    expect(humanizeError(new Error("boom")).message).toBe("链上操作没有完成，请稍后再试");
  });
});
