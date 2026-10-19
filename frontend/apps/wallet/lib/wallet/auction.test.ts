import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  auctionOrderAmount,
  authorizationProblem,
  parseAuctionAuth,
  parseSettlementProof,
  termsProblem,
} from "./auction";
import type { ApiAuctionAuth, AuctionView } from "./api";

const json: ApiAuctionAuth = {
  seller: "0x1111111111111111111111111111111111111111",
  token: "0x2222222222222222222222222222222222222222",
  auctionId: "340282366920938463463374607431768211457",
  reservePrice: "100000",
  currency: 1,
  cnyPerUsdt: 72_000,
  feePayer: 0,
  maxFeeBps: 100,
  maxBaseGas: "100000",
  maxMinFee: "1000000",
  instant: false,
  shipWindow: 3 * 86_400,
  confirmWindow: 7 * 86_400,
  listingHash: `0x${"ab".repeat(32)}`,
  expiry: 1_800_000_000,
};

describe("拍卖授权 JSON", () => {
  it("严格解析：uint256 不丢精度，expiry 是 bigint，其余小整数原样", () => {
    const a = parseAuctionAuth(json);
    expect(a?.auctionId).toBe(2n ** 128n + 1n);
    expect(a?.reservePrice).toBe(100_000n);
    expect(a?.expiry).toBe(1_800_000_000n);
    expect(a?.cnyPerUsdt).toBe(72_000);
  });

  it("任一字段格式不对、或币种与汇率对不上就整体拒绝", () => {
    const cases: [keyof ApiAuctionAuth, unknown][] = [
      ["reservePrice", "-1"],
      ["reservePrice", 100_000],
      ["seller", "0x123"],
      ["listingHash", "0x00"],
      ["feePayer", 3],
      ["maxFeeBps", 70_000],
      ["shipWindow", 1.5],
      ["instant", "false"],
      ["expiry", "1800000000"],
      // 人民币汇率超出合约范围（填成 72 或 0.72 这类错）
      ["cnyPerUsdt", 72],
      ["cnyPerUsdt", 100_001],
      ["currency", 2],
    ];
    for (const [field, bad] of cases) {
      expect(parseAuctionAuth({ ...json, [field]: bad } as ApiAuctionAuth), field).toBeNull();
    }
    // USDT 拍卖的汇率必须为 0
    expect(parseAuctionAuth({ ...json, currency: 0 })).toBeNull();
    expect(parseAuctionAuth({ ...json, currency: 0, cnyPerUsdt: 0 })).not.toBeNull();
  });

  it("成交证明同样严格", () => {
    const proof = {
      authHash: `0x${"cd".repeat(32)}`,
      winner: "0x3333333333333333333333333333333333333333",
      price: "130000",
      payDeadline: 1_800_000_000,
    };
    expect(parseSettlementProof(proof)?.price).toBe(130_000n);
    expect(parseSettlementProof({ ...proof, price: "1e5" })).toBeNull();
    expect(parseSettlementProof({ ...proof, authHash: "0x" })).toBeNull();
  });
});

interface CnyVector {
  name: string;
  priceFen: string;
  cnyPerUsdt: number;
  expect: string;
}

const vectors = JSON.parse(
  readFileSync(new URL("../../../../../test-vectors/fees.json", import.meta.url), "utf8"),
) as { cny: { toUsdt: CnyVector[] } };

describe("建单金额（与 Escrow._auctionAmount 相同）", () => {
  it.each(vectors.cny.toUsdt.map((v) => [v.name, v] as const))("人民币：%s", (_name, v) => {
    const auth = parseAuctionAuth({ ...json, cnyPerUsdt: v.cnyPerUsdt });
    if (!auth) throw new Error("fixture invalid");
    expect(auctionOrderAmount(auth, BigInt(v.priceFen))).toBe(BigInt(v.expect));
  });

  it("USDT：就是成交价", () => {
    const auth = parseAuctionAuth({ ...json, currency: 0, cnyPerUsdt: 0 });
    if (!auth) throw new Error("fixture invalid");
    expect(auctionOrderAmount(auth, 88_500_000n)).toBe(88_500_000n);
  });
});

describe("签名前核对", () => {
  const auth = parseAuctionAuth(json);
  if (!auth) throw new Error("fixture invalid");
  const authz = { auctionId: auth.auctionId.toString(), chainId: 31337, escrow: "0xEe" };
  const ctx = {
    chainId: 31337,
    escrow: "0xee",
    token: json.token.toUpperCase().replace("0X", "0x"),
    wallet: json.seller,
    nowSecs: 1_700_000_000,
  };

  it("全部对得上时没有问题（地址大小写不敏感）", () => {
    expect(authorizationProblem(authz, auth, ctx)).toBeNull();
  });

  it("网络、卖家、币种合约、编号、期限、有效期任一不对都拒签", () => {
    expect(authorizationProblem({ ...authz, chainId: 1 }, auth, ctx)).toMatch(/不属于当前网络/);
    expect(authorizationProblem({ ...authz, escrow: "0xff" }, auth, ctx)).toMatch(/不属于当前网络/);
    expect(authorizationProblem(authz, auth, { ...ctx, wallet: `0x${"9".repeat(40)}` })).toMatch(
      /卖家不是你的钱包/,
    );
    expect(authorizationProblem(authz, auth, { ...ctx, token: `0x${"9".repeat(40)}` })).toMatch(
      /不是 USDT/,
    );
    expect(authorizationProblem({ ...authz, auctionId: "1" }, auth, ctx)).toMatch(/编号对不上/);
    expect(authorizationProblem(authz, { ...auth, confirmWindow: 3600 }, ctx)).toMatch(/1–30 天/);
    // 即时发货不看发货期
    expect(authorizationProblem(authz, { ...auth, instant: true, shipWindow: 0 }, ctx)).toBeNull();
    expect(authorizationProblem(authz, auth, { ...ctx, nowSecs: 1_800_000_000 })).toMatch(/已过期/);
    // 后端没给签名域的合约地址
    expect(authorizationProblem({ ...authz, escrow: null }, auth, ctx)).toMatch(/不属于当前网络/);
  });

  // 卖家在论坛填的拍卖（后端 AuctionView，卖家本人能看到保留价）
  const view = {
    id: 1,
    currency: "cny",
    cny_per_usdt: 72_000,
    reserve_price: "100000",
    terms: { fee_payer: 0, instant: false, ship_window: 3 * 86_400, confirm_window: 7 * 86_400 },
  } as AuctionView;

  it("授权条款与论坛里填的拍卖一致才签", () => {
    expect(termsProblem(auth, view)).toBeNull();
    const bad: Partial<AuctionView>[] = [
      { reserve_price: "99999" },
      { reserve_price: null },
      { currency: "usdt", cny_per_usdt: null },
      { cny_per_usdt: 73_000 },
      { terms: { ...view.terms, fee_payer: 1 } },
      { terms: { ...view.terms, instant: true } },
      { terms: { ...view.terms, ship_window: 86_400 } },
      { terms: { ...view.terms, confirm_window: 86_400 } },
    ];
    for (const b of bad) {
      expect(termsProblem(auth, { ...view, ...b }), JSON.stringify(b)).toMatch(/不一致/);
    }
  });
});
