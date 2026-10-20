//! 拍卖规则（plan 第 6、7、9 节「拍卖」，ADR 0011）：站长可调参数、币种换算、最低加价、防狙击顺延、出价资格。
//! 纯函数，不碰网络和数据库；forum-api（出价）、forum-worker（结束与违约）、forum-sponsor（成交证明）共用。
//!
//! 金额一律是拍卖币种的最小单位：USDT 拍卖是 USDT 最小单位，人民币拍卖是分。和 USDT 比较的地方（新账号上限、
//! 订单上下限、余额门槛）先按卖家签名的汇率换成 USDT，规则与合约 `createFromAuction` 的换算相同（向上取整）。

use serde::{Deserialize, Serialize};

use crate::cny::{check_cny_per_usdt, cny_fen_to_usdt};
use crate::error::ErrorCode;
use crate::money::Usdt;

/// 拍卖最多发出几份成交证明：赢家 + 两轮次高者递补（plan 第 6 节「赢家违约」：递补一次，仍不成交再递补一次后流拍）。
pub const MAX_ROUNDS: i16 = 3;
/// 签拍卖授权的时限：开始签名后这么久内必须提交签名，否则重新开始（授权有效期按这个余量计算）。
pub const SIGN_WINDOW_SECS: u32 = 1800;
/// 授权有效期在最晚可能的付款截止时间之后再留的余量。
const EXPIRY_MARGIN_SECS: u64 = 3600;
/// 最低加价 1%（plan 第 9 节「最低加价幅度 1 USDT 或 1%（取大）」）。
const MIN_STEP_DIVISOR: u128 = 100;

/// 拍卖币种（`AuctionAuth.currency`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    Usdt,
    Cny,
}

impl Currency {
    /// 合约里的编号：`CURRENCY_USDT = 0`、`CURRENCY_CNY = 1`。
    pub fn code(self) -> u8 {
        match self {
            Self::Usdt => 0,
            Self::Cny => 1,
        }
    }

    /// JSON 里的写法（`usdt` / `cny`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usdt => "usdt",
            Self::Cny => "cny",
        }
    }

    pub fn from_code(code: i16) -> Option<Self> {
        match code {
            0 => Some(Self::Usdt),
            1 => Some(Self::Cny),
            _ => None,
        }
    }
}

/// 拍卖的计价：币种 + 人民币时卖家签名的汇率。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pricing {
    pub currency: Currency,
    /// 人民币拍卖的汇率（4 位小数定点数，50000–100000）；USDT 拍卖为 0。
    pub cny_per_usdt: u32,
}

impl Pricing {
    /// 校验币种与汇率的组合：USDT 拍卖不能带汇率，人民币拍卖的汇率必须在合约范围内。
    pub fn new(currency: Currency, cny_per_usdt: Option<u32>) -> Result<Self, ErrorCode> {
        match (currency, cny_per_usdt) {
            (Currency::Usdt, None | Some(0)) => Ok(Self {
                currency,
                cny_per_usdt: 0,
            }),
            (Currency::Cny, Some(rate)) => {
                check_cny_per_usdt(rate)?;
                Ok(Self {
                    currency,
                    cny_per_usdt: rate,
                })
            }
            _ => Err(ErrorCode::InvalidParameter),
        }
    }

    /// 拍卖币种金额折合的 USDT：与合约 `_auctionAmount` 相同（人民币向上取整）。
    pub fn to_usdt(&self, amount: u128) -> Result<Usdt, ErrorCode> {
        match self.currency {
            Currency::Usdt => Ok(Usdt(amount)),
            Currency::Cny => cny_fen_to_usdt(amount, self.cny_per_usdt),
        }
    }

    /// 1 USDT 在拍卖币种里的最小单位数（人民币向上取整到分），最低加价的绝对下限。
    pub fn one_usdt(&self) -> u128 {
        match self.currency {
            Currency::Usdt => Usdt::ONE.0,
            // 1 USDT = 汇率 ÷ 10^4 元 = 汇率 ÷ 100 分
            Currency::Cny => u128::from(self.cny_per_usdt).div_ceil(100),
        }
    }
}

/// 价格 `price` 之上的最低加价：卖家设定的加价幅度、1 USDT、价格的 1% 三者取大。
pub fn min_step(increment: u128, price: u128, pricing: &Pricing) -> u128 {
    increment
        .max(pricing.one_usdt())
        .max(price.div_ceil(MIN_STEP_DIVISOR))
}

/// 下一口最低出价：还没有出价时是起拍价，否则是当前价加最低加价。
pub fn min_next_bid(
    start_price: u128,
    increment: u128,
    current: Option<u128>,
    pricing: &Pricing,
) -> Result<u128, ErrorCode> {
    match current {
        None => Ok(start_price),
        Some(c) => c
            .checked_add(min_step(increment, c, pricing))
            .ok_or(ErrorCode::AmountOverflow),
    }
}

/// 防狙击顺延：结束前 `window` 秒内的出价把结束时间推后 `extension` 秒，但不超过 `max_end`（累计顺延上限）。
/// 时间都是 Unix 秒；返回新的结束时间（不在窗口内时不变）。
pub fn extended_end(now: i64, ends_at: i64, window: i64, extension: i64, max_end: i64) -> i64 {
    if ends_at - now <= window {
        (ends_at + extension).min(max_end).max(ends_at)
    } else {
        ends_at
    }
}

/// 出价资格（卖家挂拍时选，plan 第 9 节「拍卖默认」）：不要求 / 钱包 USDT 余额 ≥ 出价的 10% / 50% / 100% / 固定金额。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Eligibility {
    None,
    /// 余额 ≥ 出价（折合 USDT）的 `percent`%；只能是 10、50、100。
    Percent {
        percent: u8,
    },
    /// 余额 ≥ 固定金额（USDT 最小单位，十进制字符串）。
    Fixed {
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        #[serde(with = "u128_string")]
        usdt: u128,
    },
}

/// 卖家可选的余额比例档位。
pub const ELIGIBILITY_PERCENTS: [u8; 3] = [10, 50, 100];

impl Eligibility {
    pub fn check(&self, limits_cap: Usdt) -> Result<(), ErrorCode> {
        match *self {
            Self::None => Ok(()),
            Self::Percent { percent } if ELIGIBILITY_PERCENTS.contains(&percent) => Ok(()),
            Self::Fixed { usdt } if usdt > 0 && Usdt(usdt) <= limits_cap => Ok(()),
            _ => Err(ErrorCode::InvalidParameter),
        }
    }

    /// 出价 `bid_usdt`（已折合 USDT）要求的最低钱包余额；不要求时为空。
    pub fn required(&self, bid_usdt: Usdt) -> Option<Usdt> {
        match *self {
            Self::None => None,
            Self::Percent { percent } => Some(Usdt(
                bid_usdt.0.saturating_mul(u128::from(percent)).div_ceil(100),
            )),
            Self::Fixed { usdt } => Some(Usdt(usdt)),
        }
    }

    /// 数据库编码：`(kind, 参数)`，kind 0 = 不要求、1 = 比例、2 = 固定金额。
    pub fn to_db(&self) -> (i16, Option<String>) {
        match *self {
            Self::None => (0, None),
            Self::Percent { percent } => (1, Some(percent.to_string())),
            Self::Fixed { usdt } => (2, Some(usdt.to_string())),
        }
    }

    pub fn from_db(kind: i16, value: Option<&str>) -> Option<Self> {
        match (kind, value) {
            (0, _) => Some(Self::None),
            (1, Some(v)) => v.parse().ok().map(|percent| Self::Percent { percent }),
            (2, Some(v)) => v.parse().ok().map(|usdt| Self::Fixed { usdt }),
            _ => None,
        }
    }
}

mod u128_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let s = String::deserialize(d)?;
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(serde::de::Error::custom("expected a decimal string"));
        }
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// 站长在后台调整的拍卖参数（plan 第 9 节「拍卖」「拍卖风控」）；`enabled` 是上线开关（第 24–26 周开放）。
/// 缺字段按默认值；库里的值越界时收回到上下限内。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct AuctionSettings {
    /// 拍卖开关：关闭时不能挂拍、出价（已结束的拍卖照常结算，赢家照常付款）。
    pub enabled: bool,
    /// 卖家可选的拍卖时长范围（秒）。
    pub min_duration_secs: u32,
    pub max_duration_secs: u32,
    /// 结束前多少秒内的新出价触发顺延。
    pub snipe_window_secs: u32,
    /// 每次顺延多少秒。
    pub snipe_extension_secs: u32,
    /// 累计顺延上限（秒）。
    pub extension_cap_secs: u32,
    /// 赢家付款期（秒）。
    pub pay_window_secs: u32,
    /// 次高出价者递补的付款期（秒）。
    pub runner_up_window_secs: u32,
    /// 违约禁拍时长（秒）。
    pub default_ban_secs: u32,
    /// 注册不满这么多天的账号单场出价有上限。
    pub new_account_days: u32,
    /// 新账号单场出价上限（整 USDT）。
    pub new_account_cap_usdt: u32,
}

impl Default for AuctionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_duration_secs: 86_400,
            max_duration_secs: 7 * 86_400,
            snipe_window_secs: 900,
            snipe_extension_secs: 900,
            extension_cap_secs: 86_400,
            pay_window_secs: 48 * 3600,
            runner_up_window_secs: 24 * 3600,
            default_ban_secs: 30 * 86_400,
            new_account_days: 7,
            new_account_cap_usdt: 100,
        }
    }
}

impl AuctionSettings {
    /// 每项的 `(下限, 上限)`。时长最短 1 小时、最长 30 天；防狙击窗口与顺延 1–60 分钟（plan 第 9 节）。
    pub const DURATION: (u32, u32) = (3600, 30 * 86_400);
    pub const SNIPE: (u32, u32) = (60, 3600);
    pub const EXTENSION_CAP: (u32, u32) = (3600, 72 * 3600);
    pub const PAY_WINDOW: (u32, u32) = (3600, 7 * 86_400);
    pub const RUNNER_UP_WINDOW: (u32, u32) = (3600, 72 * 3600);
    pub const DEFAULT_BAN: (u32, u32) = (86_400, 365 * 86_400);
    pub const NEW_ACCOUNT_DAYS: (u32, u32) = (0, 90);
    pub const NEW_ACCOUNT_CAP: (u32, u32) = (1, 10_000);

    /// 每项的最小值与最大值（后台表单用）。
    pub fn bounds() -> (Self, Self) {
        let each = |pick: fn((u32, u32)) -> u32, enabled: bool| Self {
            enabled,
            min_duration_secs: pick(Self::DURATION),
            max_duration_secs: pick(Self::DURATION),
            snipe_window_secs: pick(Self::SNIPE),
            snipe_extension_secs: pick(Self::SNIPE),
            extension_cap_secs: pick(Self::EXTENSION_CAP),
            pay_window_secs: pick(Self::PAY_WINDOW),
            runner_up_window_secs: pick(Self::RUNNER_UP_WINDOW),
            default_ban_secs: pick(Self::DEFAULT_BAN),
            new_account_days: pick(Self::NEW_ACCOUNT_DAYS),
            new_account_cap_usdt: pick(Self::NEW_ACCOUNT_CAP),
        };
        (each(|b| b.0, false), each(|b| b.1, true))
    }

    fn fields(&self) -> [(u32, (u32, u32)); 10] {
        [
            (self.min_duration_secs, Self::DURATION),
            (self.max_duration_secs, Self::DURATION),
            (self.snipe_window_secs, Self::SNIPE),
            (self.snipe_extension_secs, Self::SNIPE),
            (self.extension_cap_secs, Self::EXTENSION_CAP),
            (self.pay_window_secs, Self::PAY_WINDOW),
            (self.runner_up_window_secs, Self::RUNNER_UP_WINDOW),
            (self.default_ban_secs, Self::DEFAULT_BAN),
            (self.new_account_days, Self::NEW_ACCOUNT_DAYS),
            (self.new_account_cap_usdt, Self::NEW_ACCOUNT_CAP),
        ]
    }

    pub fn is_valid(&self) -> bool {
        self.fields()
            .iter()
            .all(|(v, (lo, hi))| (lo..=hi).contains(&v))
            && self.min_duration_secs <= self.max_duration_secs
    }

    /// 把库里的值收回到上下限内（库里的值只经后台校验后写入，这是兜底）。
    pub fn clamped(self) -> Self {
        let c = |v: u32, (lo, hi): (u32, u32)| v.clamp(lo, hi);
        let min_duration_secs = c(self.min_duration_secs, Self::DURATION);
        Self {
            enabled: self.enabled,
            min_duration_secs,
            max_duration_secs: c(self.max_duration_secs, Self::DURATION).max(min_duration_secs),
            snipe_window_secs: c(self.snipe_window_secs, Self::SNIPE),
            snipe_extension_secs: c(self.snipe_extension_secs, Self::SNIPE),
            extension_cap_secs: c(self.extension_cap_secs, Self::EXTENSION_CAP),
            pay_window_secs: c(self.pay_window_secs, Self::PAY_WINDOW),
            runner_up_window_secs: c(self.runner_up_window_secs, Self::RUNNER_UP_WINDOW),
            default_ban_secs: c(self.default_ban_secs, Self::DEFAULT_BAN),
            new_account_days: c(self.new_account_days, Self::NEW_ACCOUNT_DAYS),
            new_account_cap_usdt: c(self.new_account_cap_usdt, Self::NEW_ACCOUNT_CAP),
        }
    }

    pub fn check_duration(&self, secs: u32) -> Result<(), ErrorCode> {
        if (self.min_duration_secs..=self.max_duration_secs).contains(&secs) {
            Ok(())
        } else {
            Err(ErrorCode::AuctionDurationInvalid)
        }
    }

    /// 新账号单场出价上限。
    pub fn new_account_cap(&self) -> Usdt {
        Usdt(u128::from(self.new_account_cap_usdt) * Usdt::ONE.0)
    }

    /// 第 `round` 轮成交证明的付款期：第 1 轮是赢家付款期，之后是递补期。
    pub fn pay_window(&self, round: i16) -> u32 {
        if round <= 1 {
            self.pay_window_secs
        } else {
            self.runner_up_window_secs
        }
    }

    /// 签名开始时（`now`，Unix 秒）算出的授权有效期：签名时限 + 时长 + 累计顺延上限 + 赢家付款期 + 两轮递补期 + 余量。
    /// 之后站长调大参数不影响已签的授权：成交证明的付款截止时间一律截到有效期内。
    pub fn auth_expiry(&self, now: u64, duration_secs: u32) -> u64 {
        now + u64::from(SIGN_WINDOW_SECS)
            + u64::from(duration_secs)
            + u64::from(self.extension_cap_secs)
            + u64::from(self.pay_window_secs)
            + 2 * u64::from(self.runner_up_window_secs)
            + EXPIRY_MARGIN_SECS
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const USDT: Pricing = Pricing {
        currency: Currency::Usdt,
        cny_per_usdt: 0,
    };

    fn cny(rate: u32) -> Pricing {
        Pricing::new(Currency::Cny, Some(rate)).unwrap()
    }

    #[test]
    fn pricing_combinations() {
        assert_eq!(Pricing::new(Currency::Usdt, None).unwrap(), USDT);
        assert_eq!(Pricing::new(Currency::Usdt, Some(0)).unwrap(), USDT);
        assert_eq!(
            Pricing::new(Currency::Usdt, Some(72_000)),
            Err(ErrorCode::InvalidParameter)
        );
        assert_eq!(
            Pricing::new(Currency::Cny, None),
            Err(ErrorCode::InvalidParameter)
        );
        assert_eq!(
            Pricing::new(Currency::Cny, Some(72)),
            Err(ErrorCode::InvalidParameter)
        );
        assert_eq!(cny(72_000).cny_per_usdt, 72_000);
        for c in [Currency::Usdt, Currency::Cny] {
            assert_eq!(serde_json::to_value(c).unwrap(), c.as_str());
            assert_eq!(Currency::from_code(c.code().into()), Some(c));
        }
    }

    #[test]
    fn conversion_rounds_up_like_the_contract() {
        assert_eq!(USDT.to_usdt(5_000_000).unwrap(), Usdt(5_000_000));
        // 576 元 ÷ 7.2 = 80 USDT 整
        assert_eq!(cny(72_000).to_usdt(57_600).unwrap(), Usdt(80_000_000));
        // 1 分 ÷ 7.2 = 1388.88… → 1389
        assert_eq!(cny(72_000).to_usdt(1).unwrap(), Usdt(1389));
    }

    #[test]
    fn one_usdt_in_each_currency() {
        assert_eq!(USDT.one_usdt(), 1_000_000);
        assert_eq!(cny(72_000).one_usdt(), 720);
        // 7.2345 元 = 723.45 分 → 724 分，不低于 1 USDT
        assert_eq!(cny(72_345).one_usdt(), 724);
        assert!(cny(72_345).to_usdt(724).unwrap() >= Usdt::ONE);
    }

    #[test]
    fn min_step_is_the_largest_of_increment_one_usdt_and_one_percent() {
        // 卖家幅度 2 USDT、价格 50 USDT：取 2 USDT
        assert_eq!(min_step(2_000_000, 50_000_000, &USDT), 2_000_000);
        // 价格 300 USDT 的 1% = 3 USDT 大于幅度
        assert_eq!(min_step(2_000_000, 300_000_000, &USDT), 3_000_000);
        // 幅度低于 1 USDT 时按 1 USDT
        assert_eq!(min_step(1, 10_000_000, &USDT), 1_000_000);
        // 1% 向上取整
        assert_eq!(min_step(0, 100_000_001, &USDT), 1_000_001);
        // 人民币：1 USDT = 720 分
        assert_eq!(min_step(100, 10_000, &cny(72_000)), 720);
        assert_eq!(min_step(100, 100_000, &cny(72_000)), 1000);
    }

    #[test]
    fn next_bid() {
        assert_eq!(
            min_next_bid(10_000_000, 1_000_000, None, &USDT).unwrap(),
            10_000_000
        );
        assert_eq!(
            min_next_bid(10_000_000, 1_000_000, Some(10_000_000), &USDT).unwrap(),
            11_000_000
        );
        assert_eq!(
            min_next_bid(0, 0, Some(u128::MAX), &USDT),
            Err(ErrorCode::AmountOverflow)
        );
    }

    #[test]
    fn anti_snipe_extends_within_the_window_up_to_the_cap() {
        let (window, ext) = (900, 900);
        let end = 10_000;
        let max_end = end + 2000;
        // 窗口外不变
        assert_eq!(extended_end(end - 901, end, window, ext, max_end), end);
        // 窗口边界内顺延
        assert_eq!(
            extended_end(end - 900, end, window, ext, max_end),
            end + 900
        );
        assert_eq!(extended_end(end - 1, end, window, ext, max_end), end + 900);
        // 顺延到上限为止
        assert_eq!(
            extended_end(end + 1500 - 1, end + 1500, window, ext, max_end),
            max_end
        );
        // 已到上限：不再顺延，也不会提前
        assert_eq!(
            extended_end(max_end - 1, max_end, window, ext, max_end),
            max_end
        );
    }

    #[test]
    fn eligibility_presets() {
        let cap = Usdt(200_000_000);
        assert!(Eligibility::None.check(cap).is_ok());
        for p in ELIGIBILITY_PERCENTS {
            assert!(Eligibility::Percent { percent: p }.check(cap).is_ok());
        }
        for bad in [0, 20, 101] {
            assert!(Eligibility::Percent { percent: bad }.check(cap).is_err());
        }
        assert!(Eligibility::Fixed { usdt: 0 }.check(cap).is_err());
        assert!(Eligibility::Fixed { usdt: cap.0 + 1 }.check(cap).is_err());
        let bid = Usdt(33_333_333);
        assert_eq!(Eligibility::None.required(bid), None);
        // 10% 向上取整
        assert_eq!(
            Eligibility::Percent { percent: 10 }.required(bid),
            Some(Usdt(3_333_334))
        );
        assert_eq!(
            Eligibility::Percent { percent: 100 }.required(bid),
            Some(bid)
        );
        assert_eq!(Eligibility::Fixed { usdt: 7 }.required(bid), Some(Usdt(7)));
        for e in [
            Eligibility::None,
            Eligibility::Percent { percent: 50 },
            Eligibility::Fixed { usdt: 12_000_000 },
        ] {
            let (k, v) = e.to_db();
            assert_eq!(Eligibility::from_db(k, v.as_deref()), Some(e));
        }
        let json = serde_json::to_value(Eligibility::Fixed { usdt: 5 }).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "fixed", "usdt": "5"}));
        assert!(
            serde_json::from_value::<Eligibility>(
                serde_json::json!({"kind": "fixed", "usdt": "-1"})
            )
            .is_err()
        );
    }

    #[test]
    fn settings_defaults_bounds_and_clamping() {
        let d = AuctionSettings::default();
        assert!(d.is_valid());
        assert!(!d.enabled, "auctions launch switched off");
        assert_eq!(d.clamped(), d);
        assert_eq!(d.new_account_cap(), Usdt(100_000_000));
        assert_eq!(d.pay_window(1), 48 * 3600);
        assert_eq!(d.pay_window(2), 24 * 3600);
        let (min, max) = AuctionSettings::bounds();
        assert!(min.is_valid() && max.is_valid());
        let bad = AuctionSettings {
            snipe_window_secs: 30,
            min_duration_secs: 7 * 86_400,
            max_duration_secs: 86_400,
            ..d
        };
        assert!(!bad.is_valid());
        let fixed = bad.clamped();
        assert!(fixed.is_valid());
        assert_eq!(fixed.snipe_window_secs, 60);
        assert_eq!(fixed.max_duration_secs, 7 * 86_400);
        let partial: AuctionSettings = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.pay_window_secs, d.pay_window_secs);
        assert!(d.check_duration(86_400).is_ok());
        assert_eq!(
            d.check_duration(3600),
            Err(ErrorCode::AuctionDurationInvalid)
        );
    }

    #[test]
    fn expiry_covers_every_possible_pay_deadline() {
        let d = AuctionSettings::default();
        let now = 1_800_000_000;
        let duration = 7 * 86_400;
        let expiry = d.auth_expiry(now, duration);
        // 最晚：签名时限用满 + 时长 + 顺延用满 + 赢家付款期 + 两轮递补
        let latest = now
            + u64::from(SIGN_WINDOW_SECS)
            + u64::from(duration)
            + u64::from(d.extension_cap_secs)
            + u64::from(d.pay_window_secs)
            + 2 * u64::from(d.runner_up_window_secs);
        assert!(expiry > latest);
    }
}
