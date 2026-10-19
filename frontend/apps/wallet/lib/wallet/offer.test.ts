import { describe, expect, it } from "vitest";
import type { ApiOffer } from "./api";
import { offerToJson, parseOffer, type Offer } from "./offer";

const offer: Offer = {
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
};

describe("报价 JSON（forum_core::eip712::Offer）", () => {
  it("往返后字段与类型完全相同（bigint 不丢精度）", () => {
    expect(parseOffer(offerToJson(offer))).toEqual(offer);
  });

  it("uint256 按十进制字符串提交，小整数是 JSON 数字", () => {
    const json = offerToJson(offer);
    expect(json.nonce).toBe("9223372036854775815");
    expect(json.amount).toBe("123456789");
    expect(json.expiry).toBe(1_800_000_000);
    expect(JSON.parse(JSON.stringify(json))).toEqual(json); // 没有 bigint，可以直接序列化
  });

  it("任一字段格式不对或缺失就整体拒绝", () => {
    const json = offerToJson(offer);
    const cases: [keyof ApiOffer, unknown][] = [
      ["amount", "-1"],
      ["amount", "1.5"],
      ["amount", 5],
      ["seller", "0x123"],
      ["instant", "true"],
      ["listingHash", "0x00"],
      ["feePayer", -1],
      ["shipWindow", 1.5],
      ["expiry", "1800000000"],
    ];
    for (const [field, bad] of cases) {
      expect(parseOffer({ ...json, [field]: bad } as ApiOffer), field).toBeNull();
    }
    const missing: Partial<ApiOffer> = { ...json };
    delete missing.nonce;
    expect(parseOffer(missing as ApiOffer)).toBeNull();
  });
});
