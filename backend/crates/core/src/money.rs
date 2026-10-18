//! USDT 金额：6 位小数，内部只存最小单位整数，不用浮点数。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ErrorCode;

/// USDT 最小单位金额（1 USDT = 1_000_000）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Usdt(pub u128);

impl Usdt {
    pub const DECIMALS: u32 = 6;
    pub const ONE: Usdt = Usdt(1_000_000);
    pub const ZERO: Usdt = Usdt(0);

    pub const fn from_units(units: u128) -> Self {
        Self(units)
    }

    pub const fn units(self) -> u128 {
        self.0
    }

    pub fn checked_add(self, rhs: Usdt) -> Result<Usdt, ErrorCode> {
        self.0
            .checked_add(rhs.0)
            .map(Usdt)
            .ok_or(ErrorCode::AmountOverflow)
    }

    pub fn checked_sub(self, rhs: Usdt) -> Result<Usdt, ErrorCode> {
        self.0
            .checked_sub(rhs.0)
            .map(Usdt)
            .ok_or(ErrorCode::AmountUnderflow)
    }

    /// `floor(self * num / den)`。
    pub fn mul_div(self, num: u128, den: u128) -> Result<Usdt, ErrorCode> {
        if den == 0 {
            return Err(ErrorCode::InvalidParameter);
        }
        self.0
            .checked_mul(num)
            .map(|v| Usdt(v / den))
            .ok_or(ErrorCode::AmountOverflow)
    }
}

impl fmt::Display for Usdt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let one = Self::ONE.0;
        write!(f, "{}.{:06}", self.0 / one, self.0 % one)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_has_six_decimals() {
        assert_eq!(Usdt(123_456_789).to_string(), "123.456789");
        assert_eq!(Usdt(5).to_string(), "0.000005");
    }

    #[test]
    fn checked_ops() {
        assert_eq!(
            Usdt(1).checked_sub(Usdt(2)),
            Err(ErrorCode::AmountUnderflow)
        );
        assert_eq!(
            Usdt(u128::MAX).checked_add(Usdt(1)),
            Err(ErrorCode::AmountOverflow)
        );
        assert_eq!(Usdt(10).mul_div(1, 0), Err(ErrorCode::InvalidParameter));
    }
}
