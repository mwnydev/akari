import { encodePacked, type Hex } from "viem";
import { afterEach, describe, expect, it, vi } from "vitest";
import { chain } from "./config";
import {
  assertSponsorFee,
  getSponsorClient,
  paymasterDataFee,
  type PaymasterFields,
} from "./sponsor";

// 生产代付走 forum-api 的 /aa/rpc；测试里给它一个绝对地址，由假的 fetch 回答
vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  AA_RPC_URL: "http://forum.test/api/v1/aa/rpc",
}));

const data = (fee: bigint) =>
  encodePacked(
    ["uint48", "uint48", "uint128", "bytes"],
    [1_800_000_120, 1_800_000_000, fee, `0x${"11".repeat(65)}`],
  );

const paymasters = [chain.contracts.FixedFeePaymaster.toLowerCase()];

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
    expect(() => assertSponsorFee(fields(50_000n), 50_000n, paymasters)).not.toThrow();
    expect(() => assertSponsorFee(fields(60_000n), 50_000n, paymasters)).toThrow(
      "代付费与确认页不一致",
    );
  });

  it("超过 1 USDT 硬上限一律拒绝，即使与展示一致", () => {
    expect(() => assertSponsorFee(fields(1_000_001n), 1_000_001n, paymasters)).toThrow();
  });

  it("Paymaster 地址大小写不影响核对", () => {
    expect(() =>
      assertSponsorFee(
        fields(
          0n,
          chain.contracts.FixedFeePaymaster.toUpperCase().replace("0X", "0x") as `0x${string}`,
        ),
        0n,
        paymasters,
      ),
    ).not.toThrow();
  });

  it("拒绝不是任何钱包版本里的 Paymaster", () => {
    expect(() =>
      assertSponsorFee(fields(0n, "0x3333333333333333333333333333333333333333"), 0n, paymasters),
    ).toThrow("代付合约地址与配置不符");
  });
});

describe("生产代付：ERC-7677 JSON-RPC（/api/v1/aa/rpc）", () => {
  afterEach(() => vi.unstubAllGlobals());

  const userOperation = {
    sender: "0x1111111111111111111111111111111111111111",
    nonce: 0n,
    callData: "0x",
    chainId: chain.chainId,
    entryPointAddress: chain.entryPoint,
  } as const;

  /** 假的 /aa/rpc：记下请求，返回给定的 result。 */
  function answer(result: object) {
    const seen: {
      url: string;
      credentials: RequestCredentials | undefined;
      body: Record<string, unknown>;
    }[] = [];
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as Record<string, unknown>;
      seen.push({ url, credentials: init.credentials, body });
      return new Response(JSON.stringify({ jsonrpc: "2.0", id: body.id, result }), {
        headers: { "content-type": "application/json" },
      });
    });
    return seen;
  }

  it("带论坛会话、把操作档位放在 context 里，返回 v0.7 的 Paymaster 字段", async () => {
    const paymasterData: Hex = data(50_000n);
    const seen = answer({
      paymaster: chain.contracts.FixedFeePaymaster,
      paymasterData,
      paymasterVerificationGasLimit: "0x1d4c0",
      paymasterPostOpGasLimit: "0x0",
    });
    const sponsor = await getSponsorClient();
    expect(await sponsor.getPaymasterStubData("createAndFund", userOperation)).toEqual({
      paymaster: chain.contracts.FixedFeePaymaster,
      paymasterData,
      paymasterVerificationGasLimit: 120_000n,
      paymasterPostOpGasLimit: 0n,
    });
    const [req] = seen;
    expect(req?.url).toBe("http://forum.test/api/v1/aa/rpc");
    expect(req?.credentials).toBe("include");
    expect(req?.body.method).toBe("pm_getPaymasterStubData");
    expect((req?.body.params as unknown[])[3]).toEqual({ action: "createAndFund" });
  });

  it("没有 postOp gas 字段时按 0 处理（FixedFeePaymaster 不实现 postOp）", async () => {
    answer({ paymaster: chain.contracts.FixedFeePaymaster, paymasterData: data(0n) });
    const sponsor = await getSponsorClient();
    const fields = await sponsor.getPaymasterData("release", userOperation);
    expect(fields.paymasterPostOpGasLimit).toBe(0n);
  });

  it("拒绝 EntryPoint v0.6 格式的返回", async () => {
    answer({ paymasterAndData: "0x1234" });
    const sponsor = await getSponsorClient();
    await expect(sponsor.getPaymasterData("release", userOperation)).rejects.toThrow(
      "代付服务返回的数据格式不对",
    );
  });
});
