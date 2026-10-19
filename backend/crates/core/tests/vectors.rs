//! 用共享测试向量 `test-vectors/fees.json` 校验费用与分期计算。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use forum_core::cny::{MAX_CNY_PER_USDT, MIN_CNY_PER_USDT};
use forum_core::{
    FeeParams, FeePayer, Usdt, claim, cny_fen_to_usdt, dispute_fee, quote, stream_net,
    stream_unlocked, usdt_to_cny_fen,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vectors {
    version: u32,
    claim_gas: String,
    fees: Vec<FeeCase>,
    streams: Vec<StreamCase>,
    cny: CnyVectors,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CnyVectors {
    min_cny_per_usdt: u32,
    max_cny_per_usdt: u32,
    to_usdt: Vec<CnyToUsdtCase>,
    reference: Vec<CnyReferenceCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CnyToUsdtCase {
    name: String,
    price_fen: String,
    cny_per_usdt: u32,
    expect: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CnyReferenceCase {
    name: String,
    usdt: String,
    cny_per_usdt: u32,
    expect_fen: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeeCase {
    name: String,
    amount: String,
    fee_payer: FeePayer,
    fee_bps: u32,
    min_fee: String,
    campaign: bool,
    base_gas: String,
    dispute_mul_bps: u32,
    expect: FeeExpect,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeeExpect {
    buyer_fee: String,
    seller_fee: String,
    buyer_gas: String,
    seller_gas: String,
    buyer_pays: String,
    dispute_fee: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamCase {
    name: String,
    amount: String,
    fee_payer: FeePayer,
    fee_bps: u32,
    min_fee: String,
    base_gas: String,
    periods: u64,
    expect: StreamExpect,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamExpect {
    unlockable_net: String,
    points: Vec<Point>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Point {
    elapsed_periods: u64,
    unlocked: String,
}

fn u(s: &str) -> Usdt {
    Usdt(s.parse().unwrap())
}

fn load() -> Vectors {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../test-vectors/fees.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn fee_vectors() {
    let v = load();
    assert_eq!(v.version, 1);
    assert!(!v.fees.is_empty());
    for c in &v.fees {
        let p = FeeParams {
            fee_bps: c.fee_bps,
            min_fee: u(&c.min_fee),
            campaign: c.campaign,
            base_gas: u(&c.base_gas),
            dispute_mul_bps: c.dispute_mul_bps,
        };
        let q = quote(u(&c.amount), c.fee_payer, &p).unwrap();
        let e = &c.expect;
        assert_eq!(q.buyer_fee, u(&e.buyer_fee), "{}: buyerFee", c.name);
        assert_eq!(q.seller_fee, u(&e.seller_fee), "{}: sellerFee", c.name);
        assert_eq!(q.buyer_gas, u(&e.buyer_gas), "{}: buyerGas", c.name);
        assert_eq!(q.seller_gas, u(&e.seller_gas), "{}: sellerGas", c.name);
        assert_eq!(q.buyer_pays, u(&e.buyer_pays), "{}: buyerPays", c.name);
        let d = dispute_fee(q.fee_total().unwrap(), &p).unwrap();
        assert_eq!(d, u(&e.dispute_fee), "{}: disputeFee", c.name);
    }
}

#[test]
fn stream_vectors() {
    let v = load();
    let claim_gas = u(&v.claim_gas);
    assert!(!v.streams.is_empty());
    for c in &v.streams {
        let p = FeeParams {
            fee_bps: c.fee_bps,
            min_fee: u(&c.min_fee),
            campaign: false,
            base_gas: u(&c.base_gas),
            dispute_mul_bps: 10_000,
        };
        let amount = u(&c.amount);
        let q = quote(amount, c.fee_payer, &p).unwrap();
        let net = stream_net(amount, &q).unwrap();
        assert_eq!(net, u(&c.expect.unlockable_net), "{}: net", c.name);
        let mut claimed = Usdt::ZERO;
        for pt in &c.expect.points {
            let unlocked = stream_unlocked(net, pt.elapsed_periods, c.periods).unwrap();
            assert_eq!(
                unlocked,
                u(&pt.unlocked),
                "{}: unlocked@{}",
                c.name,
                pt.elapsed_periods
            );
            if let Ok(got) = claim(unlocked, claimed, claim_gas) {
                assert_eq!(
                    got.checked_add(claim_gas).unwrap(),
                    unlocked.checked_sub(claimed).unwrap()
                );
                claimed = unlocked;
            }
        }
        assert_eq!(claimed, net, "{}: fully claimed at end", c.name);
    }
}

#[test]
fn cny_vectors() {
    let v = load().cny;
    assert_eq!(v.min_cny_per_usdt, MIN_CNY_PER_USDT);
    assert_eq!(v.max_cny_per_usdt, MAX_CNY_PER_USDT);
    assert!(!v.to_usdt.is_empty() && !v.reference.is_empty());
    for c in &v.to_usdt {
        let got = cny_fen_to_usdt(c.price_fen.parse().unwrap(), c.cny_per_usdt).unwrap();
        assert_eq!(got, u(&c.expect), "{}", c.name);
    }
    for c in &v.reference {
        let got = usdt_to_cny_fen(u(&c.usdt), c.cny_per_usdt).unwrap();
        assert_eq!(got, c.expect_fen.parse::<u128>().unwrap(), "{}", c.name);
    }
}
