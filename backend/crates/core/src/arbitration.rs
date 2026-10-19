//! 仲裁（plan 第 7 节「仲裁规则」、ADR 0013）：规则常量、裁决的本金分配预览，以及裁决 Passkey 要签署的内容。
//!
//! 裁决 = 链上 `resolve(orderId, toBuyerBps, bondToBuyerBps)` 的两个比例，钱只能分给买卖双方。
//! 提交裁决时用论坛裁决 Passkey 确认：WebAuthn challenge 就是 [`RulingIntent::challenge`]，
//! 它绑定案件、订单、两个比例、按链上订单算出的分配金额、理由哈希与一次性随机数，截获的断言不能用来授权另一份裁决。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::Duration;

use crate::ErrorCode;

/// 比例的分母（与合约 `BPS` 相同）。
pub const BPS: u32 = 10_000;
/// 举证期：争议发起后 72 小时。
pub const EVIDENCE_WINDOW: Duration = Duration::hours(72);
/// 裁决公示后的申诉期。
pub const APPEAL_WINDOW: Duration = Duration::hours(72);
/// 仲裁员接案后（举证期结束起算）多久不裁决就改派；forum-worker 可用环境变量调整。
pub const DEFAULT_RULING_DEADLINE: Duration = Duration::hours(48);
/// 订单金额达到这么多整 USDT 时由两人独立裁决（ADR 0013 第 2 条）。
pub const TWO_PERSON_MIN_USDT: i64 = 200;
/// `resolveTimeout` 剩余不足这么久时提醒站长。
pub const TIMEOUT_ALERT: Duration = Duration::days(3);
/// 下单快照缺失时按合约默认的 30 天算超时兜底。
pub const DEFAULT_RESOLVE_TIMEOUT_SECS: i32 = 30 * 86_400;
/// 裁决理由、申诉理由、当事人陈述的最多字数。
pub const MAX_TEXT_CHARS: usize = 2000;
/// 一条陈述最多附几张图片。
pub const MAX_STATEMENT_UPLOADS: usize = 9;

/// 谁、在哪个阶段作出的裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RulingKind {
    /// 被分配的仲裁员。
    Panel,
    /// 站长终裁（两人意见不一致，或叫停后改判）。
    Owner,
    /// 申诉复核。
    Appeal,
}

impl RulingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Owner => "owner",
            Self::Appeal => "appeal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "panel" => Some(Self::Panel),
            "owner" => Some(Self::Owner),
            "appeal" => Some(Self::Appeal),
            _ => None,
        }
    }
}

/// 争议中可分配的金额（最小单位）：一次性为订单本金；分期为未领取的可解锁净本金（合约 `net − claimed`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disputed {
    pub principal: u128,
    /// 随订单锁定的卖家保证金。
    pub bond: u128,
}

/// 按合约 `resolve` 的取整方式算出的分配（不含手续费、gas 与争议费的结算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    pub principal_to_buyer: u128,
    pub principal_to_seller: u128,
    pub bond_to_buyer: u128,
}

/// 站长提醒的标题（邮件主题与管理员群 Telegram 告警共用）；`event` 是 `forum_db::arbitration::alert_owners` 的事件名。
pub fn alert_title(event: &str) -> &str {
    match event {
        "published" => "裁决已公示",
        "owner_review" => "两名仲裁员意见不一致，等待站长终裁",
        "appealed" => "当事人已申诉，等待复核",
        "ready" => "申诉期已过，可由仲裁 Safe 执行",
        "halted" => "案件已叫停",
        "stalled" => "找不到可分配的仲裁员（全部回避或已参与），需要站长处理",
        "timeout_near" => "争议即将超时兜底（resolveTimeout 剩余不足 3 天）",
        "execution_mismatch" => "链上执行的结果与已公示的裁决不符，请立即核查仲裁 Safe",
        "chat_chain_broken" => "订单聊天哈希链校验失败（记录可能被改动），已拒绝裁决",
        other => other,
    }
}

/// 校验比例并算出分配。比例超过 100%，或订单没有保证金却要赔付保证金，返回 `INVALID_PARAMETER`。
pub fn allocate(
    d: Disputed,
    to_buyer_bps: u32,
    bond_to_buyer_bps: u32,
) -> Result<Allocation, ErrorCode> {
    if to_buyer_bps > BPS || bond_to_buyer_bps > BPS || (d.bond == 0 && bond_to_buyer_bps != 0) {
        return Err(ErrorCode::InvalidParameter);
    }
    let bps = u128::from(BPS);
    // 合约：卖家份额 = principal × (BPS − toBuyerBps) / BPS 向下取整，余下归买家。
    let principal_to_seller = d
        .principal
        .checked_mul(bps - u128::from(to_buyer_bps))
        .ok_or(ErrorCode::AmountOverflow)?
        / bps;
    let bond_to_buyer = d
        .bond
        .checked_mul(u128::from(bond_to_buyer_bps))
        .ok_or(ErrorCode::AmountOverflow)?
        / bps;
    Ok(Allocation {
        principal_to_buyer: d.principal - principal_to_seller,
        principal_to_seller,
        bond_to_buyer,
    })
}

/// 理由的 SHA-256（签署内容里只放哈希，理由原文随裁决公开）。
pub fn reason_hash(reason: &str) -> [u8; 32] {
    Sha256::digest(reason.as_bytes()).into()
}

/// 裁决 Passkey 签署的内容。字段顺序固定，金额为十进制字符串，字节为 `0x` 十六进制。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulingIntent {
    pub v: u8,
    pub case_id: i64,
    pub kind: RulingKind,
    pub chain_id: i64,
    /// 担保合约地址（`0x` + 40 位小写十六进制）。
    pub contract: String,
    pub chain_order_id: i64,
    pub to_buyer_bps: u32,
    pub bond_to_buyer_bps: u32,
    pub principal_to_buyer: String,
    pub principal_to_seller: String,
    pub bond_to_buyer: String,
    pub reason_sha256: String,
    /// 服务端为这次签署生成的 32 字节随机数。
    pub nonce: String,
}

/// 签署内容的前缀：与别处的 WebAuthn challenge 不会相同。
const INTENT_DOMAIN: &[u8] = b"akari-ruling\0";

impl RulingIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        case_id: i64,
        kind: RulingKind,
        chain_id: i64,
        contract: &[u8],
        chain_order_id: i64,
        to_buyer_bps: u32,
        bond_to_buyer_bps: u32,
        a: Allocation,
        reason: &str,
        nonce: &[u8; 32],
    ) -> Self {
        Self {
            v: 1,
            case_id,
            kind,
            chain_id,
            contract: alloy_primitives::hex::encode_prefixed(contract),
            chain_order_id,
            to_buyer_bps,
            bond_to_buyer_bps,
            principal_to_buyer: a.principal_to_buyer.to_string(),
            principal_to_seller: a.principal_to_seller.to_string(),
            bond_to_buyer: a.bond_to_buyer.to_string(),
            reason_sha256: alloy_primitives::hex::encode_prefixed(reason_hash(reason)),
            nonce: alloy_primitives::hex::encode_prefixed(nonce),
        }
    }

    /// 规范 JSON（serde_json 紧凑输出）；存进裁决记录，事后可重算 challenge。
    pub fn canonical(&self) -> String {
        // 只含整数与字符串的结构体序列化不会失败。
        serde_json::to_string(self).unwrap_or_default()
    }

    /// WebAuthn challenge：`sha256("akari-ruling\0" ‖ canonical)`。
    pub fn challenge(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(INTENT_DOMAIN);
        h.update(self.canonical().as_bytes());
        h.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const D: Disputed = Disputed {
        principal: 250_000_001,
        bond: 100_000_000,
    };

    #[test]
    fn allocation_rounds_like_the_contract() {
        let a = allocate(D, 3333, 5000).unwrap();
        // 卖家 = 250000001 × 6667 / 10000 向下取整
        assert_eq!(a.principal_to_seller, 166_675_000);
        assert_eq!(a.principal_to_buyer, 83_325_001);
        assert_eq!(a.bond_to_buyer, 50_000_000);
        assert_eq!(allocate(D, 10_000, 0).unwrap().principal_to_seller, 0);
        assert_eq!(allocate(D, 0, 0).unwrap().principal_to_buyer, 0);
    }

    #[test]
    fn allocation_rejects_out_of_range() {
        assert_eq!(allocate(D, 10_001, 0), Err(ErrorCode::InvalidParameter));
        assert_eq!(allocate(D, 0, 10_001), Err(ErrorCode::InvalidParameter));
        let no_bond = Disputed { bond: 0, ..D };
        assert_eq!(allocate(no_bond, 0, 1), Err(ErrorCode::InvalidParameter));
        assert!(allocate(no_bond, 0, 0).is_ok());
    }

    fn intent(to_buyer_bps: u32, reason: &str) -> RulingIntent {
        RulingIntent::new(
            7,
            RulingKind::Panel,
            421_614,
            &[0x44; 20],
            3,
            to_buyer_bps,
            0,
            allocate(D, to_buyer_bps, 0).unwrap(),
            reason,
            &[1; 32],
        )
    }

    #[test]
    fn challenge_binds_every_field() {
        let base = intent(5000, "各半");
        assert_eq!(
            base.canonical(),
            format!(
                r#"{{"v":1,"case_id":7,"kind":"panel","chain_id":421614,"contract":"0x{}","chain_order_id":3,"to_buyer_bps":5000,"bond_to_buyer_bps":0,"principal_to_buyer":"125000001","principal_to_seller":"125000000","bond_to_buyer":"0","reason_sha256":"0x{}","nonce":"0x{}"}}"#,
                "44".repeat(20),
                alloy_primitives::hex::encode(reason_hash("各半")),
                "01".repeat(32)
            )
        );
        assert_ne!(base.challenge(), intent(5001, "各半").challenge());
        assert_ne!(base.challenge(), intent(5000, "各半。").challenge());
        let mut other_case = base.clone();
        other_case.case_id = 8;
        assert_ne!(base.challenge(), other_case.challenge());
        let mut other_kind = base.clone();
        other_kind.kind = RulingKind::Appeal;
        assert_ne!(base.challenge(), other_kind.challenge());
        assert_eq!(base.challenge(), intent(5000, "各半").challenge());
    }
}
