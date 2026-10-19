//! 人民币计价（ADR 0011）：卖家自定的固定汇率，不接外部汇率源。
//!
//! 汇率是每 1 USDT 合多少元，4 位小数定点数（7.2000 = 72000），合约写死范围 [5.0000, 10.0000]。
//! 规则与 `test-vectors/fees.json` 的 `rules.cnyToUsdt` / `rules.cnyReference` 一致。

use crate::error::ErrorCode;
use crate::money::Usdt;

/// 汇率下限 5.0000（与合约 `MIN_CNY_PER_USDT` 一致）。
pub const MIN_CNY_PER_USDT: u32 = 50_000;
/// 汇率上限 10.0000（与合约 `MAX_CNY_PER_USDT` 一致）。
pub const MAX_CNY_PER_USDT: u32 = 100_000;

/// 10^6（USDT 最小单位）× 10^4（汇率小数位）÷ 100（每元的分数）。
const SCALE: u128 = 100_000_000;

/// 汇率在合约允许的范围内（防止把 7.2 填成 72 或 0.72）。
pub fn check_cny_per_usdt(rate: u32) -> Result<(), ErrorCode> {
    if (MIN_CNY_PER_USDT..=MAX_CNY_PER_USDT).contains(&rate) {
        Ok(())
    } else {
        Err(ErrorCode::InvalidParameter)
    }
}

/// 人民币成交价（分）按卖家汇率折合 USDT：`ceil(分 × 10^8 ÷ 汇率)`，向上取整，卖家所得不低于汇率换算值。
pub fn cny_fen_to_usdt(price_fen: u128, cny_per_usdt: u32) -> Result<Usdt, ErrorCode> {
    check_cny_per_usdt(cny_per_usdt)?;
    let scaled = price_fen
        .checked_mul(SCALE)
        .ok_or(ErrorCode::AmountOverflow)?;
    Ok(Usdt(scaled.div_ceil(u128::from(cny_per_usdt))))
}

/// USDT 标价的人民币参考价（分），只供显示：`floor(USDT × 汇率 ÷ 10^8)`。
pub fn usdt_to_cny_fen(amount: Usdt, cny_per_usdt: u32) -> Result<u128, ErrorCode> {
    check_cny_per_usdt(cny_per_usdt)?;
    Ok(amount.mul_div(u128::from(cny_per_usdt), SCALE)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_bounds_are_inclusive() {
        assert_eq!(check_cny_per_usdt(MIN_CNY_PER_USDT), Ok(()));
        assert_eq!(check_cny_per_usdt(MAX_CNY_PER_USDT), Ok(()));
        for bad in [0, 72, 49_999, 100_001] {
            assert_eq!(check_cny_per_usdt(bad), Err(ErrorCode::InvalidParameter));
            assert_eq!(cny_fen_to_usdt(100, bad), Err(ErrorCode::InvalidParameter));
            assert_eq!(
                usdt_to_cny_fen(Usdt::ONE, bad),
                Err(ErrorCode::InvalidParameter)
            );
        }
    }

    #[test]
    fn overflow_is_an_error() {
        assert_eq!(
            cny_fen_to_usdt(u128::MAX, MIN_CNY_PER_USDT),
            Err(ErrorCode::AmountOverflow)
        );
        assert_eq!(
            usdt_to_cny_fen(Usdt(u128::MAX), MIN_CNY_PER_USDT),
            Err(ErrorCode::AmountOverflow)
        );
    }
}
