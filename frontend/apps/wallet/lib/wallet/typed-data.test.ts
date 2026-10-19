import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { OFFER_TYPES } from "./typed-data";

describe("EIP-712 Offer 类型", () => {
  it("与 Escrow.sol 的 OFFER_TYPEHASH 字符串完全一致（字段名、类型、顺序）", () => {
    const source = readFileSync(
      new URL("../../../../../contracts/src/Escrow.sol", import.meta.url),
      "utf8",
    );
    const solidity = /OFFER_TYPEHASH = keccak256\(\s*"([^"]+)"/.exec(source)?.[1];
    // Offer 没有嵌套结构体，EIP-712 encodeType 就是 `Offer(类型 名称,…)`
    const encoded = `Offer(${OFFER_TYPES.Offer.map((p) => `${p.type} ${p.name}`).join(",")})`;
    expect(solidity).toBeDefined();
    expect(encoded).toBe(solidity);
  });
});
