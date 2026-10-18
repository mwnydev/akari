//! 手续费、gas 费、争议费与分期解锁计算。

use serde::{Deserialize, Serialize};

use crate::error::ErrorCode;
use crate::money::Usdt;

const BPS: u128 = 10_000;

/// 手续费与基础固定 gas 费的承担方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeePayer {
    Seller,
    Buyer,
    Half,
}

impl FeePayer {
    /// 拆成 (买家部分, 卖家部分)；各半时买家向下取整，余数归卖家。
    pub fn split(self, total: Usdt) -> (Usdt, Usdt) {
        match self {
            FeePayer::Seller => (Usdt::ZERO, total),
            FeePayer::Buyer => (total, Usdt::ZERO),
            FeePayer::Half => {
                let buyer = Usdt(total.0 / 2);
                (buyer, Usdt(total.0 - buyer.0))
            }
        }
    }
}

/// 建单时写入订单快照的费用参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeParams {
    pub fee_bps: u32,
    pub min_fee: Usdt,
    /// 免费活动期间：费率取活动费率，最低手续费失效。
    pub campaign: bool,
    pub base_gas: Usdt,
    pub dispute_mul_bps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeQuote {
    pub buyer_fee: Usdt,
    pub seller_fee: Usdt,
    pub buyer_gas: Usdt,
    pub seller_gas: Usdt,
    /// 买家建单时转入合约的总额 = 金额 + 买家手续费 + 买家 gas 费。
    pub buyer_pays: Usdt,
}

impl FeeQuote {
    pub fn fee_total(&self) -> Result<Usdt, ErrorCode> {
        self.buyer_fee.checked_add(self.seller_fee)
    }
}

fn fee_total(amount: Usdt, p: &FeeParams) -> Result<Usdt, ErrorCode> {
    let raw = amount.mul_div(u128::from(p.fee_bps), BPS)?;
    Ok(if p.campaign { raw } else { raw.max(p.min_fee) })
}

pub fn quote(amount: Usdt, payer: FeePayer, p: &FeeParams) -> Result<FeeQuote, ErrorCode> {
    let (buyer_fee, seller_fee) = payer.split(fee_total(amount, p)?);
    let (buyer_gas, seller_gas) = payer.split(p.base_gas);
    let buyer_pays = amount.checked_add(buyer_fee)?.checked_add(buyer_gas)?;
    Ok(FeeQuote {
        buyer_fee,
        seller_fee,
        buyer_gas,
        seller_gas,
        buyer_pays,
    })
}

/// 争议费 = (本单手续费，为 0 时按最低手续费) × 倍数。
pub fn dispute_fee(fee_total: Usdt, p: &FeeParams) -> Result<Usdt, ErrorCode> {
    let base = if fee_total > Usdt::ZERO {
        fee_total
    } else {
        p.min_fee
    };
    base.mul_div(u128::from(p.dispute_mul_bps), BPS)
}

/// 分期可解锁净本金 = 金额 − 卖家手续费 − 卖家基础 gas 费。
pub fn stream_net(amount: Usdt, q: &FeeQuote) -> Result<Usdt, ErrorCode> {
    amount.checked_sub(q.seller_fee)?.checked_sub(q.seller_gas)
}

/// 已解锁 = floor(净本金 × min(已过周期数, 总期数) ÷ 总期数)；余数自然归最后一期。
pub fn stream_unlocked(net: Usdt, elapsed_periods: u64, periods: u64) -> Result<Usdt, ErrorCode> {
    if periods == 0 {
        return Err(ErrorCode::InvalidParameter);
    }
    net.mul_div(
        u128::from(elapsed_periods.min(periods)),
        u128::from(periods),
    )
}

/// 本次领取：返回卖家实得（可领取 − 领取 gas 费）。要求可领取 > 领取 gas 费。
pub fn claim(unlocked: Usdt, claimed: Usdt, claim_gas: Usdt) -> Result<Usdt, ErrorCode> {
    let claimable = unlocked.checked_sub(claimed)?;
    if claimable <= claim_gas {
        return Err(ErrorCode::ClaimBelowGas);
    }
    claimable.checked_sub(claim_gas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_requires_more_than_gas() {
        let gas = Usdt(20_000);
        assert_eq!(
            claim(Usdt(20_000), Usdt::ZERO, gas),
            Err(ErrorCode::ClaimBelowGas)
        );
        assert_eq!(claim(Usdt(20_001), Usdt::ZERO, gas), Ok(Usdt(1)));
        assert_eq!(
            claim(Usdt(10), Usdt(11), gas),
            Err(ErrorCode::AmountUnderflow)
        );
    }

    #[test]
    fn zero_periods_rejected() {
        assert_eq!(
            stream_unlocked(Usdt(1), 0, 0),
            Err(ErrorCode::InvalidParameter)
        );
    }
}
