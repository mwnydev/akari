import { describe, expect, it } from "vitest";
import { bundlerPresets, endpointError, requiredPrefund, rpcPresets, topUp } from "./rules";

describe("紧急取回：节点与 ETH 预付", () => {
  it("每条链都有预填的 RPC 与 Bundler（主网用公共节点）", () => {
    expect(rpcPresets(42161).length).toBeGreaterThan(1);
    expect(bundlerPresets(42161).length).toBeGreaterThan(0);
    expect(rpcPresets(31337)[0]?.url).toMatch(/^http:\/\/127\.0\.0\.1/);
    for (const p of [...rpcPresets(42161), ...bundlerPresets(42161)]) {
      expect(endpointError(p.url)).toBeNull();
    }
  });

  it("只接受 https；本机开发链可以 http", () => {
    expect(endpointError("https://arb1.arbitrum.io/rpc")).toBeNull();
    expect(endpointError("http://127.0.0.1:8545")).toBeNull();
    expect(endpointError("http://evil.example/rpc")).toBe("只能用 https 地址");
    expect(endpointError("javascript:alert(1)")).toBe("只能用 https 地址");
    expect(endpointError("arb1.arbitrum.io")).toContain("完整的网址");
  });

  it("预付 ETH = 全部 gas 上限 × maxFeePerGas（与 EntryPoint v0.7 相同）", () => {
    const op = {
      callGasLimit: 100_000n,
      verificationGasLimit: 400_000n,
      preVerificationGas: 50_000n,
      maxFeePerGas: 20_000_000n, // 0.02 gwei
    };
    expect(requiredPrefund(op)).toBe(550_000n * 20_000_000n);
    expect(
      requiredPrefund({ ...op, paymasterVerificationGasLimit: 1n, paymasterPostOpGasLimit: 2n }),
    ).toBe(550_003n * 20_000_000n);
  });

  it("够了不用转；不够时补到预付额再多 50%", () => {
    expect(topUp(1000n, 1000n)).toBe(0n);
    expect(topUp(1000n, 0n)).toBe(1500n);
    expect(topUp(1000n, 400n)).toBe(1100n);
  });
});
