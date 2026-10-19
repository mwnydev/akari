//! 卖家报价的 EIP-712 哈希，与 `contracts/src/Escrow.sol` 的 `hashOffer` 逐字节一致。
//!
//! 报价结构只含静态类型，`abi.encode` 即每个字段左补零成 32 字节；`bytes32` 原样。
//! 交叉校验向量由 `contracts/test/OfferHashVector.t.sol` 产生，见本文件测试。

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    alloy_primitives::keccak256(data).0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("bad hex")]
    Hex,
    #[error("bad length")]
    Length,
    #[error("bad decimal")]
    Decimal,
    #[error("overflow")]
    Overflow,
}

fn parse_hex<const N: usize>(s: &str) -> Result<[u8; N], ParseError> {
    let s = s.strip_prefix("0x").ok_or(ParseError::Hex)?;
    if s.len() != N * 2 {
        return Err(ParseError::Length);
    }
    let mut out = [0u8; N];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16).ok_or(ParseError::Hex)?;
        let lo = (chunk[1] as char).to_digit(16).ok_or(ParseError::Hex)?;
        out[i] = u8::try_from(hi * 16 + lo).map_err(|_| ParseError::Hex)?;
    }
    Ok(out)
}

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    f.write_str("0x")?;
    for b in bytes {
        write!(f, "{b:02x}")?;
    }
    Ok(())
}

/// 为「0x 十六进制字符串」形式的定长字节类型实现解析、显示与 serde。
macro_rules! hex_bytes {
    ($name:ident, $n:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name(pub [u8; $n]);

        impl $name {
            pub const ZERO: Self = Self([0u8; $n]);

            pub fn is_zero(&self) -> bool {
                self.0 == [0u8; $n]
            }
        }

        impl FromStr for $name {
            type Err = ParseError;
            fn from_str(s: &str) -> Result<Self, ParseError> {
                parse_hex::<$n>(s).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(f, &self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(f, &self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

hex_bytes!(
    Address,
    20,
    "20 字节地址，JSON 里是 `0x` + 40 位十六进制（大小写均可，输出小写）。"
);
hex_bytes!(B256, 32, "32 字节哈希，JSON 里是 `0x` + 64 位十六进制。");

/// 无符号 256 位整数（大端字节）。JSON 里是十进制字符串，避免 JavaScript 精度丢失。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct U256(pub [u8; 32]);

impl U256 {
    pub const ZERO: Self = Self([0u8; 32]);

    pub fn from_u128(v: u128) -> Self {
        let mut b = [0u8; 32];
        b[16..].copy_from_slice(&v.to_be_bytes());
        Self(b)
    }

    /// 能放进 u128 时返回其值（USDT 金额都在这个范围内）。
    pub fn to_u128(self) -> Option<u128> {
        if self.0[..16].iter().any(|&b| b != 0) {
            return None;
        }
        let mut lo = [0u8; 16];
        lo.copy_from_slice(&self.0[16..]);
        Some(u128::from_be_bytes(lo))
    }
}

impl FromStr for U256 {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        if s.is_empty() || s.len() > 78 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ParseError::Decimal);
        }
        // 小端 4 × u64：每位数字乘 10 再加上。
        let mut limbs = [0u64; 4];
        for d in s.bytes().map(|b| u64::from(b - b'0')) {
            let mut carry = u128::from(d);
            for limb in &mut limbs {
                let v = u128::from(*limb) * 10 + carry;
                *limb = v as u64;
                carry = v >> 64;
            }
            if carry != 0 {
                return Err(ParseError::Overflow);
            }
        }
        let mut b = [0u8; 32];
        for (i, limb) in limbs.iter().enumerate() {
            b[(3 - i) * 8..(4 - i) * 8].copy_from_slice(&limb.to_be_bytes());
        }
        Ok(Self(b))
    }
}

impl fmt::Display for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(v) = self.to_u128() {
            return write!(f, "{v}");
        }
        // 大于 u128 的值（只有 nonce 会出现）：反复除以 10。
        let mut limbs = [0u64; 4];
        for (i, limb) in limbs.iter_mut().enumerate() {
            let mut w = [0u8; 8];
            w.copy_from_slice(&self.0[(3 - i) * 8..(4 - i) * 8]);
            *limb = u64::from_be_bytes(w);
        }
        let mut digits = Vec::with_capacity(78);
        while limbs.iter().any(|&l| l != 0) {
            let mut rem = 0u128;
            for limb in limbs.iter_mut().rev() {
                let v = (rem << 64) | u128::from(*limb);
                *limb = (v / 10) as u64;
                rem = v % 10;
            }
            digits.push(b'0' + rem as u8);
        }
        digits.reverse();
        f.write_str(&String::from_utf8_lossy(&digits))
    }
}

impl fmt::Debug for U256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for U256 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for U256 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// 与 `Escrow.Offer` 字段一一对应、同序。JSON 字段名与 EIP-712 typed data 的 `message` 相同（驼峰）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Offer {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0x1111111111111111111111111111111111111111"))]
    pub seller: Address,
    /// 全 0 = 公开报价，任何人可付款。
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub buyer: Address,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub token: Address,
    /// USDT 最小单位，十进制字符串。
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "45000000"))]
    pub amount: U256,
    /// 0 = 卖家、1 = 买家、2 = 各半（`FeeMath.Payer`）。
    pub fee_payer: u8,
    pub max_fee_bps: u16,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub max_base_gas: U256,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub max_min_fee: U256,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub bond: U256,
    pub late_slash_bps: u16,
    pub instant: bool,
    pub ship_window: u32,
    pub confirm_window: u32,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub listing_hash: B256,
    /// Unix 秒。
    pub expiry: u64,
    pub max_fills: u32,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub nonce: U256,
}

/// 签名域：`EIP712("AkariEscrow", "1")` + chainId + 合约地址，测试网、主网、新旧合约之间不能重放。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Domain {
    pub chain_id: u64,
    pub verifying_contract: Address,
}

pub const DOMAIN_NAME: &str = "AkariEscrow";
pub const DOMAIN_VERSION: &str = "1";
const DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
/// 与 `Escrow.OFFER_TYPEHASH` 的原文相同。
pub const OFFER_TYPE: &str = "Offer(address seller,address buyer,address token,uint256 amount,uint8 feePayer,uint16 maxFeeBps,uint256 maxBaseGas,uint256 maxMinFee,uint256 bond,uint16 lateSlashBps,bool instant,uint32 shipWindow,uint32 confirmWindow,bytes32 listingHash,uint64 expiry,uint32 maxFills,uint256 nonce)";

fn word_u64(v: u64) -> [u8; 32] {
    U256::from_u128(u128::from(v)).0
}

fn word_addr(a: &Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&a.0);
    w
}

impl Domain {
    pub fn separator(&self) -> [u8; 32] {
        let mut buf = Vec::with_capacity(32 * 5);
        buf.extend_from_slice(&keccak256(DOMAIN_TYPE.as_bytes()));
        buf.extend_from_slice(&keccak256(DOMAIN_NAME.as_bytes()));
        buf.extend_from_slice(&keccak256(DOMAIN_VERSION.as_bytes()));
        buf.extend_from_slice(&word_u64(self.chain_id));
        buf.extend_from_slice(&word_addr(&self.verifying_contract));
        keccak256(&buf)
    }
}

impl Offer {
    /// `keccak256(abi.encode(OFFER_TYPEHASH, offer))`
    pub fn struct_hash(&self) -> [u8; 32] {
        let words: [[u8; 32]; 18] = [
            keccak256(OFFER_TYPE.as_bytes()),
            word_addr(&self.seller),
            word_addr(&self.buyer),
            word_addr(&self.token),
            self.amount.0,
            word_u64(self.fee_payer.into()),
            word_u64(self.max_fee_bps.into()),
            self.max_base_gas.0,
            self.max_min_fee.0,
            self.bond.0,
            word_u64(self.late_slash_bps.into()),
            word_u64(self.instant.into()),
            word_u64(self.ship_window.into()),
            word_u64(self.confirm_window.into()),
            self.listing_hash.0,
            word_u64(self.expiry),
            word_u64(self.max_fills.into()),
            self.nonce.0,
        ];
        keccak256(words.as_flattened())
    }

    /// `Escrow.hashOffer`：`keccak256(0x1901 ‖ domainSeparator ‖ structHash)`，即卖家签名的摘要。
    pub fn hash(&self, domain: &Domain) -> B256 {
        let mut buf = [0u8; 66];
        buf[..2].copy_from_slice(b"\x19\x01");
        buf[2..34].copy_from_slice(&domain.separator());
        buf[34..].copy_from_slice(&self.struct_hash());
        B256(keccak256(&buf))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn keccak_empty() {
        assert_eq!(
            B256(keccak256(b"")).to_string(),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn u256_roundtrip() {
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        for s in [
            "0",
            "1",
            "45000000",
            "340282366920938463463374607431768211456",
            max,
        ] {
            assert_eq!(s.parse::<U256>().unwrap().to_string(), s);
        }
        assert_eq!(
            "115792089237316195423570985008687907853269984665640564039457584007913129639936"
                .parse::<U256>(),
            Err(ParseError::Overflow)
        );
        assert!("".parse::<U256>().is_err());
        assert!("-1".parse::<U256>().is_err());
        assert!("1e6".parse::<U256>().is_err());
        assert_eq!(
            "5000000".parse::<U256>().unwrap().to_u128(),
            Some(5_000_000)
        );
        assert_eq!(max.parse::<U256>().unwrap().to_u128(), None);
    }

    #[test]
    fn address_parse() {
        let a: Address = "0xAbCdEf0000000000000000000000000000000001"
            .parse()
            .unwrap();
        assert_eq!(a.to_string(), "0xabcdef0000000000000000000000000000000001");
        assert!(
            "abcdef0000000000000000000000000000000001"
                .parse::<Address>()
                .is_err()
        );
        assert!("0xabcd".parse::<Address>().is_err());
        assert!(
            "0xzz00000000000000000000000000000000000000"
                .parse::<Address>()
                .is_err()
        );
    }

    /// 来自 `forge test --match-contract OfferHashVector -vv`（contracts/test/OfferHashVector.t.sol）。
    /// 合约侧同一测试断言了相同的常量，任意一侧改动编码都会让两边测试之一失败。
    pub(crate) fn vector() -> (Offer, Domain) {
        let offer = Offer {
            seller: "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            buyer: "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            token: "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            amount: "45000000".parse().unwrap(),
            fee_payer: 2,
            max_fee_bps: 150,
            max_base_gas: "100000".parse().unwrap(),
            max_min_fee: "1000000".parse().unwrap(),
            bond: "10000000".parse().unwrap(),
            late_slash_bps: 2500,
            instant: false,
            ship_window: 259_200,
            confirm_window: 604_800,
            listing_hash: "0xabababababababababababababababababababababababababababababababab"
                .parse()
                .unwrap(),
            expiry: 1_800_000_000,
            max_fills: 1,
            nonce: "115792089237316195423570985008687907853269984665640564039457584007913129639935"
                .parse()
                .unwrap(),
        };
        let domain = Domain {
            chain_id: 421_614,
            verifying_contract: "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
        };
        (offer, domain)
    }

    #[test]
    fn typehash_matches_contract() {
        // Escrow.OFFER_TYPEHASH()
        assert_eq!(
            B256(keccak256(OFFER_TYPE.as_bytes())).to_string(),
            VECTOR_TYPEHASH
        );
    }

    #[test]
    fn offer_hash_matches_solidity() {
        let (offer, domain) = vector();
        assert_eq!(
            B256(domain.separator()).to_string(),
            VECTOR_DOMAIN_SEPARATOR
        );
        assert_eq!(offer.hash(&domain).to_string(), VECTOR_OFFER_HASH);
    }

    #[test]
    fn json_is_typed_data_message() {
        let (offer, _) = vector();
        let json = serde_json::to_value(offer).unwrap();
        assert_eq!(json["feePayer"], 2);
        assert_eq!(json["amount"], "45000000");
        assert_eq!(json["listingHash"].as_str().unwrap().len(), 66);
        let back: Offer = serde_json::from_value(json).unwrap();
        assert_eq!(back, offer);
    }

    const VECTOR_TYPEHASH: &str =
        "0x04f9fa206ae158239be1142f748338be0840a0fb1258b71de33d031ee90ee59b";
    const VECTOR_DOMAIN_SEPARATOR: &str =
        "0x7b6062683cdf5554ed30e3116f4e43854a6286baade707e86541123091f6eb81";
    const VECTOR_OFFER_HASH: &str =
        "0x8dbd5dcf642988d717b308cc79507ae5a6bdfc8126f74b9f8182001bababeeb8";
}
