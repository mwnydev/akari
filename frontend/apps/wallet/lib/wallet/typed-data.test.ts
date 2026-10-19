import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { AUCTION_TYPES, OFFER_TYPES, PROOF_TYPES } from "./typed-data";

const source = readFileSync(
  new URL("../../../../../contracts/src/Escrow.sol", import.meta.url),
  "utf8",
);

/** Solidity 里 `NAME = keccak256("…")` 的类型字符串（可能跨行）。 */
function typehash(name: string): string | undefined {
  return new RegExp(`${name} =\\s*keccak256\\(\\s*"([^"]+)"`).exec(source)?.[1];
}

/** 没有嵌套结构体时 EIP-712 encodeType 就是 `名称(类型 字段,…)`。 */
function encodeType(name: string, fields: readonly { name: string; type: string }[]): string {
  return `${name}(${fields.map((p) => `${p.type} ${p.name}`).join(",")})`;
}

describe("EIP-712 类型与 Escrow.sol 的 TYPEHASH 字符串完全一致（字段名、类型、顺序）", () => {
  it("Offer", () => {
    expect(typehash("OFFER_TYPEHASH")).toBeDefined();
    expect(encodeType("Offer", OFFER_TYPES.Offer)).toBe(typehash("OFFER_TYPEHASH"));
  });

  it("AuctionAuth", () => {
    expect(typehash("AUCTION_TYPEHASH")).toBeDefined();
    expect(encodeType("AuctionAuth", AUCTION_TYPES.AuctionAuth)).toBe(typehash("AUCTION_TYPEHASH"));
  });

  it("SettlementProof", () => {
    expect(typehash("PROOF_TYPEHASH")).toBeDefined();
    expect(encodeType("SettlementProof", PROOF_TYPES.SettlementProof)).toBe(
      typehash("PROOF_TYPEHASH"),
    );
  });
});
