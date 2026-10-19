import { describe, expect, it } from "vitest";
import { decodeSignedOffer, encodeSignedOffer, type SignedOffer } from "./offer";

const signed: SignedOffer = {
  offer: {
    seller: "0x1111111111111111111111111111111111111111",
    buyer: "0x0000000000000000000000000000000000000000",
    token: "0x2222222222222222222222222222222222222222",
    amount: 123_456_789n,
    feePayer: 2,
    maxFeeBps: 100,
    maxBaseGas: 100_000n,
    maxMinFee: 1_000_000n,
    bond: 0n,
    lateSlashBps: 0,
    instant: true,
    shipWindow: 86_400,
    confirmWindow: 259_200,
    listingHash: `0x${"ab".repeat(32)}`,
    expiry: 1_800_000_000n,
    maxFills: 1,
    nonce: 2n ** 63n + 7n,
  },
  signature: `0x${"cd".repeat(100)}`,
};

describe("报价链接编码", () => {
  it("往返后字段与类型完全相同（bigint 不丢精度）", () => {
    expect(decodeSignedOffer(encodeSignedOffer(signed))).toEqual(signed);
  });

  it("编码结果是 URL 安全的", () => {
    expect(encodeSignedOffer(signed)).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it.each([
    ["不是 base64", "%%%"],
    ["不是 JSON", btoa("hello")],
    ["缺签名", btoa(JSON.stringify({ o: {} }))],
  ])("拒绝畸形输入：%s", (_, code) => {
    expect(decodeSignedOffer(code)).toBeNull();
  });

  it("任一字段格式不对就整体拒绝", () => {
    const json = JSON.parse(
      atob(encodeSignedOffer(signed).replaceAll("-", "+").replaceAll("_", "/")),
    );
    const cases: [string, string][] = [
      ["amount", "-1"],
      ["amount", "1.5"],
      ["seller", "0x123"],
      ["instant", "yes"],
      ["listingHash", "0x00"],
    ];
    for (const [field, bad] of cases) {
      const tampered = { ...json, o: { ...json.o, [field]: bad } };
      expect(decodeSignedOffer(btoa(JSON.stringify(tampered))), field).toBeNull();
    }
    const missing = { ...json.o };
    delete missing.nonce;
    expect(decodeSignedOffer(btoa(JSON.stringify({ ...json, o: missing })))).toBeNull();
  });
});
