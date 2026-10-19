//! 报价与交易帖条款的范围校验：镜像 `Escrow._checkTerms` 与 `EscrowConfig` 的常量，让界面在签名前就失败，
//! 而不是等买家付款时链上回滚（plan 第 6、9 节）。链上仍是最终裁判，这里只做提前拦截。

use crate::eip712::{Address, Offer, U256};
use crate::error::ErrorCode;
use crate::money::Usdt;

/// `Escrow.MIN_WINDOW` / `MAX_WINDOW`：发货期限与确认期 24 小时–30 天。
pub const MIN_WINDOW: u32 = 24 * 3600;
pub const MAX_WINDOW: u32 = 30 * 24 * 3600;
const BPS: u16 = 10_000;
/// `EscrowConfig` 写死的上限：费率 ≤ 10%、最低手续费 ≤ 5 USDT、固定 gas 费 ≤ 1 USDT。
pub const MAX_FEE_BPS: u16 = 1000;
pub const MAX_MIN_FEE: Usdt = Usdt(5_000_000);
pub const MAX_BASE_GAS: Usdt = Usdt(1_000_000);
/// 报价有效期上限（链下规则；plan 第 9 节公开报价默认 30 天）。
pub const MAX_OFFER_TTL_SECS: u64 = 30 * 24 * 3600;
/// 报价至少还要有效这么久，否则买家来不及付款。
pub const MIN_OFFER_TTL_SECS: u64 = 300;
/// 每份报价最多可成交次数（链下规则，防止公开报价被误设成无限）。
pub const MAX_FILLS: u32 = 10_000;

/// 当前链上参数（站长经金库 Safe 设置；默认值见 plan 第 9 节）。签名前对照，避免签出必然 `FeeLimit` 的报价。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeLimits {
    pub min_order: Usdt,
    pub order_cap: Usdt,
    pub fee_bps: u16,
    pub min_fee: Usdt,
    pub base_gas: Usdt,
}

impl Default for TradeLimits {
    fn default() -> Self {
        Self {
            min_order: Usdt(5_000_000),
            order_cap: Usdt(200_000_000),
            fee_bps: 100,
            min_fee: Usdt(1_000_000),
            base_gas: Usdt(100_000),
        }
    }
}

fn usdt(v: U256) -> Result<Usdt, ErrorCode> {
    v.to_u128().map(Usdt).ok_or(ErrorCode::AmountOutOfRange)
}

impl TradeLimits {
    pub fn check_amount(&self, amount: Usdt) -> Result<(), ErrorCode> {
        if amount < self.min_order || amount > self.order_cap {
            return Err(ErrorCode::AmountOutOfRange);
        }
        Ok(())
    }
}

/// 发货期限（非即时发货时）与确认期都在 24 小时–30 天内。
pub fn check_windows(
    instant: bool,
    ship_window: u32,
    confirm_window: u32,
) -> Result<(), ErrorCode> {
    let ok = |w: u32| (MIN_WINDOW..=MAX_WINDOW).contains(&w);
    if !ok(confirm_window) || (!instant && !ok(ship_window)) {
        return Err(ErrorCode::WindowOutOfRange);
    }
    Ok(())
}

/// 保证金不超过订单金额，违约比例不超过 100%。
pub fn check_bond(amount: Usdt, bond: Usdt, late_slash_bps: u16) -> Result<(), ErrorCode> {
    if bond > amount || late_slash_bps > BPS {
        return Err(ErrorCode::BondInvalid);
    }
    Ok(())
}

/// 报价的全部静态检查。`now` 为 Unix 秒；`token` 为本链 USDT 地址。
pub fn check_offer(
    o: &Offer,
    limits: &TradeLimits,
    token: &Address,
    now: u64,
) -> Result<(), ErrorCode> {
    if o.seller.is_zero() || o.seller == o.buyer || o.token != *token || o.fee_payer > 2 {
        return Err(ErrorCode::InvalidParameter);
    }
    if o.max_fills == 0 || o.max_fills > MAX_FILLS {
        return Err(ErrorCode::InvalidParameter);
    }
    let amount = usdt(o.amount)?;
    limits.check_amount(amount)?;
    check_bond(amount, usdt(o.bond)?, o.late_slash_bps)?;
    check_windows(o.instant, o.ship_window, o.confirm_window)?;
    // 卖家接受的上限低于当前收费：合约会以 FeeLimit 拒绝；高于合约写死的上限则没有意义。
    let max_base_gas = usdt(o.max_base_gas).map_err(|_| ErrorCode::FeeLimitInvalid)?;
    let max_min_fee = usdt(o.max_min_fee).map_err(|_| ErrorCode::FeeLimitInvalid)?;
    if o.max_fee_bps < limits.fee_bps
        || max_base_gas < limits.base_gas
        || max_min_fee < limits.min_fee
        || o.max_fee_bps > MAX_FEE_BPS
        || max_base_gas > MAX_BASE_GAS
        || max_min_fee > MAX_MIN_FEE
    {
        return Err(ErrorCode::FeeLimitInvalid);
    }
    if o.expiry < now.saturating_add(MIN_OFFER_TTL_SECS)
        || o.expiry > now.saturating_add(MAX_OFFER_TTL_SECS)
    {
        return Err(ErrorCode::OfferExpiryInvalid);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::eip712::B256;

    const NOW: u64 = 1_800_000_000;

    fn token() -> Address {
        "0x3333333333333333333333333333333333333333"
            .parse()
            .unwrap()
    }

    fn offer() -> Offer {
        Offer {
            seller: "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            buyer: Address::ZERO,
            token: token(),
            amount: U256::from_u128(45_000_000),
            fee_payer: 0,
            max_fee_bps: 100,
            max_base_gas: U256::from_u128(100_000),
            max_min_fee: U256::from_u128(1_000_000),
            bond: U256::ZERO,
            late_slash_bps: 0,
            instant: false,
            ship_window: 3 * 86_400,
            confirm_window: 7 * 86_400,
            listing_hash: B256::ZERO,
            expiry: NOW + 86_400,
            max_fills: 1,
            nonce: U256::from_u128(7),
        }
    }

    fn check(o: &Offer) -> Result<(), ErrorCode> {
        check_offer(o, &TradeLimits::default(), &token(), NOW)
    }

    #[test]
    fn default_offer_passes() {
        check(&offer()).unwrap();
    }

    #[test]
    fn amount_bounds() {
        let mut o = offer();
        o.amount = U256::from_u128(5_000_000);
        check(&o).unwrap();
        o.amount = U256::from_u128(4_999_999);
        assert_eq!(check(&o), Err(ErrorCode::AmountOutOfRange));
        o.amount = U256::from_u128(200_000_000);
        check(&o).unwrap();
        o.amount = U256::from_u128(200_000_001);
        assert_eq!(check(&o), Err(ErrorCode::AmountOutOfRange));
        o.amount = U256([0xff; 32]);
        assert_eq!(check(&o), Err(ErrorCode::AmountOutOfRange));
    }

    #[test]
    fn window_bounds() {
        let mut o = offer();
        o.ship_window = MIN_WINDOW - 1;
        assert_eq!(check(&o), Err(ErrorCode::WindowOutOfRange));
        // 即时发货不看发货期限
        o.instant = true;
        check(&o).unwrap();
        o.confirm_window = MAX_WINDOW + 1;
        assert_eq!(check(&o), Err(ErrorCode::WindowOutOfRange));
        o.confirm_window = MAX_WINDOW;
        check(&o).unwrap();
    }

    #[test]
    fn bond_and_slash() {
        let mut o = offer();
        o.bond = o.amount;
        o.late_slash_bps = 10_000;
        check(&o).unwrap();
        o.bond = U256::from_u128(45_000_001);
        assert_eq!(check(&o), Err(ErrorCode::BondInvalid));
        o.bond = U256::ZERO;
        o.late_slash_bps = 10_001;
        assert_eq!(check(&o), Err(ErrorCode::BondInvalid));
    }

    #[test]
    fn fee_limits_must_cover_current_quote() {
        let mut o = offer();
        o.max_fee_bps = 99;
        assert_eq!(check(&o), Err(ErrorCode::FeeLimitInvalid));
        o.max_fee_bps = 1001;
        assert_eq!(check(&o), Err(ErrorCode::FeeLimitInvalid));
        let mut o = offer();
        o.max_base_gas = U256::from_u128(99_999);
        assert_eq!(check(&o), Err(ErrorCode::FeeLimitInvalid));
        let mut o = offer();
        o.max_min_fee = U256::from_u128(5_000_001);
        assert_eq!(check(&o), Err(ErrorCode::FeeLimitInvalid));
    }

    #[test]
    fn expiry_and_identity() {
        let mut o = offer();
        o.expiry = NOW + 299;
        assert_eq!(check(&o), Err(ErrorCode::OfferExpiryInvalid));
        o.expiry = NOW + MAX_OFFER_TTL_SECS + 1;
        assert_eq!(check(&o), Err(ErrorCode::OfferExpiryInvalid));
        let mut o = offer();
        o.buyer = o.seller;
        assert_eq!(check(&o), Err(ErrorCode::InvalidParameter));
        let mut o = offer();
        o.token = Address::ZERO;
        assert_eq!(check(&o), Err(ErrorCode::InvalidParameter));
        let mut o = offer();
        o.fee_payer = 3;
        assert_eq!(check(&o), Err(ErrorCode::InvalidParameter));
        let mut o = offer();
        o.max_fills = 0;
        assert_eq!(check(&o), Err(ErrorCode::InvalidParameter));
    }
}
