import { arbitrumSepoliaChain, localChain } from "@akari/contracts";
import { describe, expect, it } from "vitest";
import { selectChain } from "./config";

describe("selectChain (NEXT_PUBLIC_WALLET_CHAIN)", () => {
  it("defaults to the local AA dev chain", () => {
    expect(selectChain(undefined)).toBe(localChain);
    expect(selectChain("")).toBe(localChain);
    expect(selectChain("local")).toBe(localChain);
  });

  it("selects Arbitrum Sepolia with its emitted wallet init versions", () => {
    const c = selectChain("arbitrum-sepolia");
    expect(c).toBe(arbitrumSepoliaChain);
    expect(c.chainId).toBe(421614);
    expect(c.walletInitVersions.map((v) => v.version)).toEqual([1]);
    expect(c.walletInitVersions[0]?.paymaster).toBe(c.contracts.FixedFeePaymaster);
  });

  it("rejects unknown names instead of silently falling back", () => {
    for (const name of ["arbitrum", "Arbitrum-Sepolia", "toString", "__proto__"]) {
      expect(() => selectChain(name)).toThrow(/NEXT_PUBLIC_WALLET_CHAIN/);
    }
  });
});
