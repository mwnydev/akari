//! 仲裁（plan 第 7 节「仲裁规则」、ADR 0013）：案件、分配与回避、裁决、申诉、叫停、当事人陈述、裁决 Passkey。
//!
//! 流程状态（`arbitration_cases.status`）：
//!
//! ```text
//! reviewing ──仲裁员裁决（一人；两人且一致）──▶ appeal_window ──72 小时无人申诉──▶ ready ──Resolved 上链──▶ 关闭
//!     │                                          │    ▲                         ▲   │
//!     └──两人不一致──▶ owner_review ──站长终裁──┘    │                         │   └──站长叫停──▶ halted
//!                                                  申诉 ──▶ appealed ──复核──▶ ┘       （恢复或改判）
//! ```
//!
//! 订单的链上结局与流程状态分开记（`closed_at` / `close_reason`，见 [`sync_order`]），区块回滚只影响前者。
//! 所有状态变化写只增审计日志；系统操作的 `actor_id` 为 [`SYSTEM_ACTOR`]。
//! 每次状态变化都先锁案件行（[`lock_case`]），同一案件的操作严格串行。

use forum_core::arbitration::{
    APPEAL_WINDOW, DEFAULT_RESOLVE_TIMEOUT_SECS, Disputed, EVIDENCE_WINDOW, RulingKind,
    TIMEOUT_ALERT, TWO_PERSON_MIN_USDT, alert_title,
};
use serde_json::{Value, json};
use sqlx::{PgExecutor, Postgres, Transaction};
use time::{Duration, OffsetDateTime};

use crate::orders::STATE_DISPUTED;
use crate::{audit, outbox, staff};

/// 审计日志里 forum-indexer / forum-worker 的操作者。用户 ID 从 1 开始，0 不会与任何人重合。
pub const SYSTEM_ACTOR: i64 = 0;

pub const STATUS_REVIEWING: &str = "reviewing";
pub const STATUS_OWNER_REVIEW: &str = "owner_review";
pub const STATUS_APPEAL_WINDOW: &str = "appeal_window";
pub const STATUS_APPEALED: &str = "appealed";
pub const STATUS_READY: &str = "ready";
pub const STATUS_HALTED: &str = "halted";

pub const ROLE_PANEL: &str = "panel";
pub const ROLE_APPEAL: &str = "appeal";

pub const CLOSE_EXECUTED: &str = "executed";
pub const CLOSE_SETTLED: &str = "settled";
pub const CLOSE_REVERTED: &str = "reverted";

/// `passkeys.purpose`：论坛裁决 Passkey（与钱包的 `wallet::PURPOSE_WALLET` 是两组凭证）。
pub const PURPOSE_RULING: i16 = 3;

/// 发件箱：给站长的提醒邮件（站内通知另有 `notify.arbitration_*`）。
pub const EMAIL_ALERT: &str = "email.arbitration_alert";

const STATE_NONE: i16 = 0;
const STATE_FUNDED: i16 = 1;
const STATE_SETTLED: i16 = 5;

/// 案件与它的订单（链上字段来自 escrow_orders）。
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub id: i64,
    pub order_id: i64,
    pub status: String,
    pub panel_size: i16,
    pub evidence_until: OffsetDateTime,
    pub final_ruling_id: Option<i64>,
    pub published_at: Option<OffsetDateTime>,
    pub appeal_until: Option<OffsetDateTime>,
    pub appeal_by: Option<i64>,
    pub appeal_reason: Option<String>,
    pub appealed_at: Option<OffsetDateTime>,
    pub halted_from: Option<String>,
    pub halted_by: Option<i64>,
    pub halt_reason: Option<String>,
    pub ready_at: Option<OffsetDateTime>,
    pub closed_at: Option<OffsetDateTime>,
    pub close_reason: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub buyer_id: Option<i64>,
    pub seller_id: Option<i64>,
    pub chain_id: i64,
    pub contract: Vec<u8>,
    pub chain_order_id: i64,
    pub mode: i16,
    pub order_state: i16,
    pub amount: String,
    pub bond: Option<String>,
    /// 分期：可解锁净本金（下单快照）；一次性为空。
    pub net: Option<String>,
    /// 分期：已领取（合约 `claimed`）。
    pub released_amount: String,
    pub token_decimals: i16,
    pub disputed_at: Option<OffsetDateTime>,
    pub resolve_timeout_secs: i32,
    pub executed_to_buyer_bps: Option<i32>,
    pub executed_bond_to_buyer_bps: Option<i32>,
}

fn amount(s: &str) -> Result<u128, sqlx::Error> {
    s.parse()
        .map_err(|_| sqlx::Error::Decode(format!("bad amount {s}").into()))
}

impl Case {
    pub fn is_party(&self, user_id: i64) -> bool {
        self.buyer_id == Some(user_id) || self.seller_id == Some(user_id)
    }

    /// 裁决人数（1 或 2）。
    pub fn panel(&self) -> usize {
        if self.panel_size == 2 { 2 } else { 1 }
    }

    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    /// 争议中可分配的金额：一次性为本金，分期为未领取的可解锁净本金（与合约 `resolve` 相同）。
    pub fn disputed(&self) -> Result<Disputed, sqlx::Error> {
        let bond = self.bond.as_deref().map_or(Ok(0), amount)?;
        let principal = match (self.mode, &self.net) {
            (0, _) => amount(&self.amount)?,
            (1, Some(net)) => amount(net)?
                .checked_sub(amount(&self.released_amount)?)
                .ok_or_else(|| sqlx::Error::Decode("claimed exceeds net".into()))?,
            // 分期订单没有下单快照就算不出争议中的金额：宁可报错也不按订单金额估。
            _ => return Err(sqlx::Error::Decode("stream order without snapshot".into())),
        };
        Ok(Disputed { principal, bond })
    }

    /// `resolveTimeout` 可调用的时间（争议时间 + 下单快照里的超时）。
    pub fn timeout_at(&self) -> Option<OffsetDateTime> {
        self.disputed_at
            .map(|t| t + Duration::seconds(self.resolve_timeout_secs.into()))
    }
}

/// 锁住案件行（事务内），之后读到的就是最新状态。
pub async fn lock_case(tx: &mut Transaction<'_, Postgres>, id: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar!(
        "SELECT id FROM arbitration_cases WHERE id = $1 FOR UPDATE",
        id
    )
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

pub async fn case_by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Case>, sqlx::Error> {
    sqlx::query_file_as!(
        Case,
        "queries/arbitration_case.sql",
        id,
        DEFAULT_RESOLVE_TIMEOUT_SECS
    )
    .fetch_optional(db)
    .await
}

pub async fn case_id_by_order(
    db: impl PgExecutor<'_>,
    order_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM arbitration_cases WHERE order_id = $1",
        order_id
    )
    .fetch_optional(db)
    .await
}

// ───────────────────────────── 权限 ─────────────────────────────

/// 能否接案（站长或管理员，见迁移里的 `arbitrator_eligible`）。
pub async fn is_eligible(db: impl PgExecutor<'_>, user_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT arbitrator_eligible($1) AS "ok!""#, user_id)
        .fetch_one(db)
        .await
}

/// 是否站长（终裁、叫停、查看全部案件；ADR 0013 第 1 条）。
pub async fn is_owner(db: impl PgExecutor<'_>, user_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM staff_roles WHERE user_id = $1 AND role = $2) AS "ok!""#,
        user_id,
        staff::ROLE_OWNER
    )
    .fetch_one(db)
    .await
}

/// 是否因与当事人的关系回避（见迁移里的 `arbitrator_recused`）。
pub async fn is_recused(
    db: impl PgExecutor<'_>,
    case_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT arbitrator_recused($1, $2) AS "ok!""#,
        case_id,
        user_id
    )
    .fetch_one(db)
    .await
}

/// 是否参与过本案（被分配过或作出过裁决）：申诉复核只能由没参与的人做。
pub async fn took_part(
    db: impl PgExecutor<'_>,
    case_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT (EXISTS (SELECT 1 FROM arbitration_assignments WHERE case_id = $1 AND arbitrator_id = $2)
                OR EXISTS (SELECT 1 FROM arbitration_rulings WHERE case_id = $1 AND arbitrator_id = $2)) AS "ok!""#,
        case_id,
        user_id
    )
    .fetch_one(db)
    .await
}

/// 订单聊天与订单详情的仲裁方读权限：该订单案件的站长，或被分配且没被改派走的仲裁员。
pub async fn can_review_order(
    db: impl PgExecutor<'_>,
    order_id: i64,
    user_id: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM arbitration_cases c
             WHERE c.order_id = $1 AND (
               EXISTS (SELECT 1 FROM staff_roles s WHERE s.user_id = $2 AND s.role = $3)
               OR EXISTS (SELECT 1 FROM arbitration_assignments a
                          WHERE a.case_id = c.id AND a.arbitrator_id = $2
                            AND a.ended_reason IS DISTINCT FROM 'reassigned'))
           ) AS "ok!""#,
        order_id,
        user_id,
        staff::ROLE_OWNER
    )
    .fetch_one(db)
    .await
}

// ───────────────────────────── 读取 ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub id: i64,
    pub case_id: i64,
    pub arbitrator_id: i64,
    pub role: String,
    pub assigned_at: OffsetDateTime,
    pub due_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
    pub ended_reason: Option<String>,
}

impl Assignment {
    /// 还在岗（未提交、未被改派）。
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }

    /// 参与了本案（在岗或已提交裁决；被改派走的不算）。
    pub fn counts(&self) -> bool {
        self.ended_reason.as_deref() != Some("reassigned")
    }
}

pub async fn assignments(
    db: impl PgExecutor<'_>,
    case_id: i64,
) -> Result<Vec<Assignment>, sqlx::Error> {
    sqlx::query_as!(
        Assignment,
        "SELECT id, case_id, arbitrator_id, role, assigned_at, due_at, ended_at, ended_reason
         FROM arbitration_assignments WHERE case_id = $1 ORDER BY id",
        case_id
    )
    .fetch_all(db)
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruling {
    pub id: i64,
    pub case_id: i64,
    pub arbitrator_id: i64,
    pub assignment_id: Option<i64>,
    pub kind: String,
    pub to_buyer_bps: i32,
    pub bond_to_buyer_bps: i32,
    pub principal_to_buyer: String,
    pub principal_to_seller: String,
    pub bond_to_buyer: String,
    pub reason: String,
    pub chat_head: Vec<u8>,
    pub chat_count: i64,
    pub signed_intent: String,
    pub passkey_id: i64,
    pub created_at: OffsetDateTime,
}

impl Ruling {
    fn split(&self) -> (i32, i32) {
        (self.to_buyer_bps, self.bond_to_buyer_bps)
    }
}

pub async fn rulings(db: impl PgExecutor<'_>, case_id: i64) -> Result<Vec<Ruling>, sqlx::Error> {
    sqlx::query_as!(
        Ruling,
        r#"SELECT id, case_id, arbitrator_id, assignment_id, kind, to_buyer_bps, bond_to_buyer_bps,
                  principal_to_buyer::text AS "principal_to_buyer!", principal_to_seller::text AS "principal_to_seller!",
                  bond_to_buyer::text AS "bond_to_buyer!", reason, chat_head, chat_count, signed_intent, passkey_id,
                  created_at
           FROM arbitration_rulings WHERE case_id = $1 ORDER BY id"#,
        case_id
    )
    .fetch_all(db)
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub id: i64,
    pub user_id: i64,
    pub body: String,
    pub upload_ids: Vec<i64>,
    pub created_at: OffsetDateTime,
}

pub async fn statements(
    db: impl PgExecutor<'_>,
    case_id: i64,
) -> Result<Vec<Statement>, sqlx::Error> {
    sqlx::query_as!(
        Statement,
        "SELECT id, user_id, body, upload_ids, created_at
         FROM arbitration_statements WHERE case_id = $1 ORDER BY id",
        case_id
    )
    .fetch_all(db)
    .await
}

/// 当事人证据图片的地址（处理完成的各尺寸，按陈述顺序）。
pub async fn evidence_links(
    db: impl PgExecutor<'_>,
    case_id: i64,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT v->>'url' AS "url!"
           FROM arbitration_statements s
           CROSS JOIN LATERAL unnest(s.upload_ids) WITH ORDINALITY AS u(id, n)
           JOIN uploads up ON up.id = u.id AND up.user_id = s.user_id AND up.status = 'done'
           CROSS JOIN LATERAL jsonb_array_elements(up.variants) AS v
           WHERE s.case_id = $1 AND v ? 'url'
           ORDER BY s.id, u.n, (v->>'width')::int"#,
        case_id
    )
    .fetch_all(db)
    .await
}

/// 仲裁员待办的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub case_id: i64,
    pub order_id: i64,
    pub status: String,
    pub role: String,
    pub due_at: OffsetDateTime,
    pub evidence_until: OffsetDateTime,
}

/// 我在岗的分配（未关闭的案件）。
pub async fn queue(db: impl PgExecutor<'_>, user_id: i64) -> Result<Vec<QueueItem>, sqlx::Error> {
    sqlx::query_as!(
        QueueItem,
        "SELECT c.id AS case_id, c.order_id, c.status, a.role, a.due_at, c.evidence_until
         FROM arbitration_assignments a JOIN arbitration_cases c ON c.id = a.case_id
         WHERE a.arbitrator_id = $1 AND a.ended_at IS NULL AND c.closed_at IS NULL
         ORDER BY a.due_at, c.id",
        user_id
    )
    .fetch_all(db)
    .await
}

/// 站长要处理或盯着的案件：待终裁、待复核、已叫停、待 Safe 执行，以及找不到人接案的、临近 `resolveTimeout` 兜底的。
pub async fn owner_queue(db: impl PgExecutor<'_>) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, status FROM arbitration_cases
         WHERE closed_at IS NULL
           AND (status IN ('owner_review', 'appealed', 'halted', 'ready')
                OR stalled_alerted_at IS NOT NULL OR timeout_alerted_at IS NOT NULL)
         ORDER BY id"
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.status)).collect())
}

/// 待仲裁 Safe 执行的案件（未关闭）。
pub async fn ready_case_ids(db: impl PgExecutor<'_>) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM arbitration_cases WHERE status = 'ready' AND closed_at IS NULL ORDER BY id"
    )
    .fetch_all(db)
    .await
}

/// 每位仲裁员的工作量与按时率（计酬只看这些，绝不看裁决方向；ADR 0013 第 2 条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workload {
    pub arbitrator_id: i64,
    pub assigned: i64,
    pub ruled: i64,
    /// 在期限内提交的。
    pub on_time: i64,
    /// 超时被改派的。
    pub reassigned: i64,
    /// 未关闭案件里还在岗的。
    pub open: i64,
    /// 站长终裁与站长复核（没有分配记录）。
    pub owner_rulings: i64,
}

pub async fn workload(
    db: impl PgExecutor<'_>,
    since: OffsetDateTime,
) -> Result<Vec<Workload>, sqlx::Error> {
    sqlx::query_as!(
        Workload,
        r#"WITH a AS (
             SELECT a.arbitrator_id,
                    count(*) AS assigned,
                    count(*) FILTER (WHERE a.ended_reason = 'ruled') AS ruled,
                    count(*) FILTER (WHERE a.ended_reason = 'ruled' AND a.ended_at <= a.due_at) AS on_time,
                    count(*) FILTER (WHERE a.ended_reason = 'reassigned') AS reassigned,
                    count(*) FILTER (WHERE a.ended_at IS NULL AND c.closed_at IS NULL) AS open
             FROM arbitration_assignments a JOIN arbitration_cases c ON c.id = a.case_id
             WHERE a.assigned_at >= $1
             GROUP BY a.arbitrator_id
           ), o AS (
             SELECT arbitrator_id, count(*) AS owner_rulings FROM arbitration_rulings
             WHERE assignment_id IS NULL AND created_at >= $1
             GROUP BY arbitrator_id
           )
           SELECT coalesce(a.arbitrator_id, o.arbitrator_id) AS "arbitrator_id!",
                  coalesce(a.assigned, 0) AS "assigned!", coalesce(a.ruled, 0) AS "ruled!",
                  coalesce(a.on_time, 0) AS "on_time!", coalesce(a.reassigned, 0) AS "reassigned!",
                  coalesce(a.open, 0) AS "open!", coalesce(o.owner_rulings, 0) AS "owner_rulings!"
           FROM a FULL JOIN o ON o.arbitrator_id = a.arbitrator_id
           ORDER BY 1"#,
        since
    )
    .fetch_all(db)
    .await
}

// ───────────────────────────── 通知、提醒与审计 ─────────────────────────────

fn target(case_id: i64) -> String {
    format!("arbitration_case:{case_id}")
}

/// 写审计日志（同一事务）。`action` 统一以 `arbitration.` 开头。
pub async fn audit(
    db: impl PgExecutor<'_>,
    actor_id: i64,
    action: &str,
    case_id: i64,
    reason: Option<&str>,
    detail: &Value,
) -> Result<(), sqlx::Error> {
    audit::append(db, actor_id, action, Some(&target(case_id)), reason, detail).await?;
    Ok(())
}

/// 站内通知 `system`（`event` = `arbitration_{event}`），经发件箱投递。
pub async fn notify(
    db: impl PgExecutor<'_>,
    user_id: i64,
    event: &str,
    case_id: i64,
) -> Result<(), sqlx::Error> {
    outbox::enqueue(
        db,
        &format!("notify.arbitration_{event}"),
        &json!({ "to": user_id, "case_id": case_id }),
        outbox::PRIORITY_NORMAL,
    )
    .await?;
    Ok(())
}

/// 推送给站长（ADR 0013 第 5 条「每份裁决连同理由和证据链接自动推送站长」及其他需要站长处理的事）：
/// 每位站长一条站内通知 + 一封邮件，另发一条管理员群 Telegram 告警（只有案件号与标题：理由、证据等案情
/// 不出站外，看详情要登录论坛）。所有站长提醒都只经过这一个函数。
pub async fn alert_owners(
    tx: &mut Transaction<'_, Postgres>,
    event: &str,
    case_id: i64,
    detail: Value,
) -> Result<(), sqlx::Error> {
    let owners = sqlx::query!(
        "SELECT u.id, u.email FROM users u
         WHERE EXISTS (SELECT 1 FROM staff_roles s WHERE s.user_id = u.id AND s.role = $1)
         ORDER BY u.id",
        staff::ROLE_OWNER
    )
    .fetch_all(&mut **tx)
    .await?;
    for o in owners {
        notify(&mut **tx, o.id, event, case_id).await?;
        outbox::enqueue(
            &mut **tx,
            EMAIL_ALERT,
            &json!({ "to": o.email, "event": event, "case_id": case_id, "detail": detail }),
            outbox::PRIORITY_NORMAL,
        )
        .await?;
    }
    outbox::staff_alert(
        &mut **tx,
        &format!("arbitration.{event}"),
        &format!("仲裁案件 #{case_id}：{}", alert_title(event)),
    )
    .await?;
    Ok(())
}

async fn touch_status(
    tx: &mut Transaction<'_, Postgres>,
    case_id: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE arbitration_cases SET status = $2, updated_at = now() WHERE id = $1",
        case_id,
        status
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ───────────────────────────── indexer：随订单链上状态开关案件 ─────────────────────────────

/// [`sync_order`] 对案件做了什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Synced {
    Opened(i64),
    Reopened(i64),
    Closed(i64, &'static str),
}

/// 订单行改写后（indexer 同一事务）按链上状态同步案件，可重复调用、结果只取决于订单当前状态：
/// - 争议中：没有案件就新建（金额 ≥ 200 USDT 两人裁决，举证期从争议的区块时间起算）；已关闭的重新打开（区块回滚）；
/// - 离开争议：关闭（`Resolved` = executed，回到已付款或订单被撤销 = reverted，其他结束方式 = settled）。
///   链上执行的比例与已公示的生效裁决不符，或案件还不能执行就被执行了，提醒站长。
pub async fn sync_order(
    tx: &mut Transaction<'_, Postgres>,
    order_id: i64,
) -> Result<Option<Synced>, sqlx::Error> {
    let Some(case_id) = case_id_by_order(&mut **tx, order_id).await? else {
        let order = sqlx::query!(
            r#"SELECT o.state, o.disputed_at,
                      o.amount >= $2::bigint * power(10::numeric, c.token_decimals) AS "two_person!"
               FROM escrow_orders o JOIN chains c ON c.chain_id = o.chain_id WHERE o.id = $1"#,
            order_id,
            TWO_PERSON_MIN_USDT
        )
        .fetch_one(&mut **tx)
        .await?;
        if order.state != STATE_DISPUTED {
            return Ok(None);
        }
        let evidence_until =
            order.disputed_at.unwrap_or_else(OffsetDateTime::now_utc) + EVIDENCE_WINDOW;
        let panel: i16 = if order.two_person { 2 } else { 1 };
        let id = sqlx::query_scalar!(
            "INSERT INTO arbitration_cases (order_id, panel_size, evidence_until) VALUES ($1, $2, $3) RETURNING id",
            order_id,
            panel,
            evidence_until
        )
        .fetch_one(&mut **tx)
        .await?;
        audit(
            &mut **tx,
            SYSTEM_ACTOR,
            "arbitration.opened",
            id,
            None,
            &json!({ "order_id": order_id, "panel_size": panel }),
        )
        .await?;
        return Ok(Some(Synced::Opened(id)));
    };
    lock_case(tx, case_id).await?;
    let Some(case) = case_by_id(&mut **tx, case_id).await? else {
        return Ok(None);
    };
    if case.order_state == STATE_DISPUTED {
        if case.is_open() {
            return Ok(None);
        }
        sqlx::query!(
            "UPDATE arbitration_cases SET closed_at = NULL, close_reason = NULL, updated_at = now() WHERE id = $1",
            case_id
        )
        .execute(&mut **tx)
        .await?;
        audit(
            &mut **tx,
            SYSTEM_ACTOR,
            "arbitration.reopened",
            case_id,
            Some("chain reorg"),
            &json!({ "order_id": order_id, "previous": case.close_reason }),
        )
        .await?;
        return Ok(Some(Synced::Reopened(case_id)));
    }
    let reason = match case.order_state {
        STATE_NONE | STATE_FUNDED => CLOSE_REVERTED,
        STATE_SETTLED if case.executed_to_buyer_bps.is_some() => CLOSE_EXECUTED,
        _ => CLOSE_SETTLED,
    };
    if case.close_reason.as_deref() == Some(reason) {
        return Ok(None);
    }
    sqlx::query!(
        "UPDATE arbitration_cases SET closed_at = now(), close_reason = $2, updated_at = now() WHERE id = $1",
        case_id,
        reason
    )
    .execute(&mut **tx)
    .await?;
    let executed = (reason == CLOSE_EXECUTED).then(|| {
        (
            case.executed_to_buyer_bps.unwrap_or_default(),
            case.executed_bond_to_buyer_bps.unwrap_or_default(),
        )
    });
    audit(
        &mut **tx,
        SYSTEM_ACTOR,
        "arbitration.closed",
        case_id,
        Some(reason),
        &json!({ "order_id": order_id, "status": case.status, "executed": executed }),
    )
    .await?;
    if let Some(done) = executed {
        let published = match case.final_ruling_id {
            Some(id) if case.status == STATUS_READY => rulings(&mut **tx, case_id)
                .await?
                .into_iter()
                .find(|r| r.id == id)
                .map(|r| r.split()),
            _ => None,
        };
        if published != Some(done) {
            alert_owners(
                tx,
                "execution_mismatch",
                case_id,
                json!({ "executed": done, "published": published, "status": case.status }),
            )
            .await?;
        }
    }
    Ok(Some(Synced::Closed(case_id, reason)))
}

// ───────────────────────────── worker：分配、改派、申诉期、超时提醒 ─────────────────────────────

/// 随机选出可接案的人：可接案（`arbitrator_eligible`）、不回避（`arbitrator_recused`）、没参与过本案。
/// 两人裁决的案件不抽站长：站长负责意见不一致时的终裁（ADR 0013 第 5 条）。
/// `gen_random_uuid()` 用 PostgreSQL 的强随机数，分配不可预测。
async fn candidates(
    tx: &mut Transaction<'_, Postgres>,
    case_id: i64,
    exclude_owners: bool,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT e.user_id AS "user_id!" FROM (
             SELECT DISTINCT user_id FROM staff_roles WHERE arbitrator_eligible(user_id)
           ) e
           WHERE NOT arbitrator_recused($1, e.user_id)
             AND NOT ($2 AND EXISTS (SELECT 1 FROM staff_roles s WHERE s.user_id = e.user_id AND s.role = $4))
             AND NOT EXISTS (SELECT 1 FROM arbitration_assignments a WHERE a.case_id = $1 AND a.arbitrator_id = e.user_id)
             AND NOT EXISTS (SELECT 1 FROM arbitration_rulings r WHERE r.case_id = $1 AND r.arbitrator_id = e.user_id)
           ORDER BY gen_random_uuid()
           LIMIT $3"#,
        case_id,
        exclude_owners,
        limit,
        staff::ROLE_OWNER
    )
    .fetch_all(&mut **tx)
    .await
}

async fn assign(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    user_id: i64,
    role: &str,
    due_at: OffsetDateTime,
) -> Result<i64, sqlx::Error> {
    let id = sqlx::query_scalar!(
        "INSERT INTO arbitration_assignments (case_id, arbitrator_id, role, due_at) VALUES ($1, $2, $3, $4) RETURNING id",
        case.id,
        user_id,
        role,
        due_at
    )
    .fetch_one(&mut **tx)
    .await?;
    audit(
        &mut **tx,
        SYSTEM_ACTOR,
        "arbitration.assigned",
        case.id,
        None,
        &json!({ "arbitrator_id": user_id, "role": role, "due_at": due_at.unix_timestamp() }),
    )
    .await?;
    notify(&mut **tx, user_id, "assigned", case.id).await?;
    Ok(id)
}

/// 找不到人接案：提醒站长一次（之后分配成功会清掉标记）。
async fn stalled(tx: &mut Transaction<'_, Postgres>, case: &Case) -> Result<bool, sqlx::Error> {
    let first = sqlx::query!(
        "UPDATE arbitration_cases SET stalled_alerted_at = now() WHERE id = $1 AND stalled_alerted_at IS NULL",
        case.id
    )
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    if first {
        alert_owners(tx, "stalled", case.id, json!({ "status": case.status })).await?;
    }
    Ok(first)
}

async fn unstall(tx: &mut Transaction<'_, Postgres>, case_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE arbitration_cases SET stalled_alerted_at = NULL WHERE id = $1 AND stalled_alerted_at IS NOT NULL",
        case_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 一轮 worker 处理的结果（日志与测试用）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    pub assigned: usize,
    pub reassigned: usize,
    pub ready: usize,
    pub stalled: usize,
    pub timeout_alerts: usize,
}

impl Tick {
    pub fn total(&self) -> usize {
        self.assigned + self.reassigned + self.ready + self.stalled + self.timeout_alerts
    }
}

/// 一轮处理（每个案件一个事务）：补齐裁决 / 复核人、超时改派、申诉期到期转为可执行、`resolveTimeout` 临近提醒。
/// `now` 由调用方给出（测试可以推进时间）；`deadline` 是接案后的裁决期限。
pub async fn tick(
    db: &crate::PgPool,
    now: OffsetDateTime,
    deadline: Duration,
) -> Result<Tick, sqlx::Error> {
    let mut out = Tick::default();
    let due = sqlx::query_scalar!(
        r#"SELECT c.id FROM arbitration_cases c JOIN escrow_orders o ON o.id = c.order_id
           WHERE c.closed_at IS NULL AND (
             (c.status = 'appeal_window' AND c.appeal_until <= $1)
             OR c.status IN ('reviewing', 'appealed')
             OR (c.timeout_alerted_at IS NULL
                 AND o.disputed_at + make_interval(secs => coalesce((o.snapshot->>'resolve_timeout')::int, $2) - $3)
                     <= $1)
           )
           ORDER BY c.id"#,
        now,
        DEFAULT_RESOLVE_TIMEOUT_SECS,
        TIMEOUT_ALERT.whole_seconds() as f64
    )
    .fetch_all(db)
    .await?;
    for id in due {
        let mut tx = db.begin().await?;
        // 其他 worker 正在处理同一案件时跳过，下一轮再看。
        let locked = sqlx::query_scalar!(
            "SELECT id FROM arbitration_cases WHERE id = $1 AND closed_at IS NULL FOR UPDATE SKIP LOCKED",
            id
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(case) = (match locked {
            Some(_) => case_by_id(&mut *tx, id).await?,
            None => None,
        }) else {
            continue;
        };
        tick_case(&mut tx, &case, now, deadline, &mut out).await?;
        tx.commit().await?;
    }
    Ok(out)
}

async fn tick_case(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    now: OffsetDateTime,
    deadline: Duration,
    out: &mut Tick,
) -> Result<(), sqlx::Error> {
    match case.status.as_str() {
        STATUS_REVIEWING => {
            let start = now.max(case.evidence_until);
            staff_case(
                tx,
                case,
                ROLE_PANEL,
                case.panel(),
                start + deadline,
                now,
                out,
            )
            .await?;
        }
        STATUS_APPEALED => {
            staff_case(tx, case, ROLE_APPEAL, 1, now + deadline, now, out).await?;
        }
        STATUS_APPEAL_WINDOW if case.appeal_until.is_some_and(|t| t <= now) => {
            sqlx::query!(
                "UPDATE arbitration_cases SET status = 'ready', ready_at = $2, updated_at = now() WHERE id = $1",
                case.id,
                now
            )
            .execute(&mut **tx)
            .await?;
            audit(
                &mut **tx,
                SYSTEM_ACTOR,
                "arbitration.ready",
                case.id,
                Some("appeal window passed"),
                &json!({}),
            )
            .await?;
            alert_owners(tx, "ready", case.id, json!({})).await?;
            out.ready += 1;
        }
        _ => {}
    }
    if let Some(at) = case.timeout_at()
        && at - TIMEOUT_ALERT <= now
    {
        let first = sqlx::query!(
            "UPDATE arbitration_cases SET timeout_alerted_at = $2 WHERE id = $1 AND timeout_alerted_at IS NULL",
            case.id,
            now
        )
        .execute(&mut **tx)
        .await?
        .rows_affected()
            > 0;
        if first {
            alert_owners(
                tx,
                "timeout_near",
                case.id,
                json!({ "timeout_at": at.unix_timestamp(), "status": case.status }),
            )
            .await?;
            out.timeout_alerts += 1;
        }
    }
    Ok(())
}

/// 让案件在 `role` 上有 `want` 个在岗或已提交的人：先把超时未提交的改派给新人，再补空位。
/// 找不到人时超时的人留在岗位上（来得及提交仍然有效），并提醒站长。
async fn staff_case(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    role: &str,
    want: usize,
    due_at: OffsetDateTime,
    now: OffsetDateTime,
    out: &mut Tick,
) -> Result<(), sqlx::Error> {
    let exclude_owners = role == ROLE_PANEL && case.panel_size == 2;
    let current: Vec<Assignment> = assignments(&mut **tx, case.id)
        .await?
        .into_iter()
        .filter(|a| a.role == role && a.counts())
        .collect();
    let mut short = false;
    for late in current.iter().filter(|a| a.is_active() && a.due_at < now) {
        let Some(&next) = candidates(tx, case.id, exclude_owners, 1).await?.first() else {
            short = true;
            continue;
        };
        sqlx::query!(
            "UPDATE arbitration_assignments SET ended_at = $2, ended_reason = 'reassigned' WHERE id = $1",
            late.id,
            now
        )
        .execute(&mut **tx)
        .await?;
        audit(
            &mut **tx,
            SYSTEM_ACTOR,
            "arbitration.reassigned",
            case.id,
            Some("ruling deadline passed"),
            &json!({ "from": late.arbitrator_id, "to": next, "role": role }),
        )
        .await?;
        notify(&mut **tx, late.arbitrator_id, "reassigned", case.id).await?;
        assign(tx, case, next, role, due_at).await?;
        out.reassigned += 1;
    }
    let missing = want.saturating_sub(current.len());
    if missing > 0 {
        let picked = candidates(
            tx,
            case.id,
            exclude_owners,
            i64::try_from(missing).unwrap_or(0),
        )
        .await?;
        short |= picked.len() < missing;
        for user in picked {
            assign(tx, case, user, role, due_at).await?;
            out.assigned += 1;
        }
    }
    if short {
        // 申诉找不到未参与的管理员时由站长复核（站长本人也参与过的案件只能等超时兜底）。
        if stalled(tx, case).await? {
            out.stalled += 1;
        }
    } else {
        unstall(tx, case.id).await?;
    }
    Ok(())
}

// ───────────────────────────── API：陈述、裁决、申诉、叫停 ─────────────────────────────

pub async fn add_statement(
    tx: &mut Transaction<'_, Postgres>,
    case_id: i64,
    user_id: i64,
    body: &str,
    upload_ids: &[i64],
) -> Result<i64, sqlx::Error> {
    let id = sqlx::query_scalar!(
        "INSERT INTO arbitration_statements (case_id, user_id, body, upload_ids) VALUES ($1, $2, $3, $4) RETURNING id",
        case_id,
        user_id,
        body,
        upload_ids
    )
    .fetch_one(&mut **tx)
    .await?;
    audit(
        &mut **tx,
        user_id,
        "arbitration.statement",
        case_id,
        None,
        &json!({ "statement_id": id, "uploads": upload_ids }),
    )
    .await?;
    Ok(id)
}

/// `upload_ids` 里属于 `user_id` 且已处理完成的个数（陈述只能附自己的图片）。
pub async fn own_done_uploads(
    db: impl PgExecutor<'_>,
    user_id: i64,
    upload_ids: &[i64],
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM uploads WHERE id = ANY($1) AND user_id = $2 AND status = 'done'"#,
        upload_ids,
        user_id
    )
    .fetch_one(db)
    .await
}

/// 一份待写入的裁决（调用方已校验权限、Passkey 断言与聊天哈希链）。
#[derive(Debug, Clone)]
pub struct NewRuling<'a> {
    pub case_id: i64,
    pub arbitrator_id: i64,
    pub assignment_id: Option<i64>,
    pub kind: RulingKind,
    pub to_buyer_bps: i32,
    pub bond_to_buyer_bps: i32,
    pub principal_to_buyer: String,
    pub principal_to_seller: String,
    pub bond_to_buyer: String,
    pub reason: &'a str,
    pub chat_head: &'a [u8],
    pub chat_count: i64,
    pub signed_intent: &'a str,
    pub challenge: &'a [u8],
    pub passkey_id: i64,
    pub assertion: &'a Value,
}

/// 写入裁决后案件去了哪里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 两人裁决还差另一人。
    Waiting,
    /// 两人意见不一致，转站长终裁。
    Disagreed,
    /// 已公示，进入申诉期。
    Published,
    /// 申诉复核结果：已公示且为最终裁决，可以执行。
    Final,
}

/// 写入一份裁决并推进状态（调用方已锁案件行并确认状态允许这份裁决）。
pub async fn submit_ruling(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    r: &NewRuling<'_>,
) -> Result<(i64, Outcome), sqlx::Error> {
    let id = sqlx::query_scalar!(
        "INSERT INTO arbitration_rulings (case_id, arbitrator_id, assignment_id, kind, to_buyer_bps, bond_to_buyer_bps,
                                          principal_to_buyer, principal_to_seller, bond_to_buyer, reason, chat_head,
                                          chat_count, signed_intent, challenge, passkey_id, assertion)
         VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric, $8::text::numeric, $9::text::numeric, $10, $11, $12, $13,
                 $14, $15, $16)
         RETURNING id",
        r.case_id,
        r.arbitrator_id,
        r.assignment_id,
        r.kind.as_str(),
        r.to_buyer_bps,
        r.bond_to_buyer_bps,
        r.principal_to_buyer,
        r.principal_to_seller,
        r.bond_to_buyer,
        r.reason,
        r.chat_head,
        r.chat_count,
        r.signed_intent,
        r.challenge,
        r.passkey_id,
        r.assertion
    )
    .fetch_one(&mut **tx)
    .await?;
    if let Some(a) = r.assignment_id {
        sqlx::query!(
            "UPDATE arbitration_assignments SET ended_at = now(), ended_reason = 'ruled' WHERE id = $1",
            a
        )
        .execute(&mut **tx)
        .await?;
    }
    let split = (r.to_buyer_bps, r.bond_to_buyer_bps);
    audit(
        &mut **tx,
        r.arbitrator_id,
        "arbitration.ruling",
        case.id,
        Some(r.reason),
        &json!({ "ruling_id": id, "kind": r.kind.as_str(), "to_buyer_bps": split.0, "bond_to_buyer_bps": split.1 }),
    )
    .await?;
    let outcome = match r.kind {
        RulingKind::Panel => {
            let panel: Vec<Ruling> = rulings(&mut **tx, case.id)
                .await?
                .into_iter()
                .filter(|x| x.kind == ROLE_PANEL)
                .collect();
            if panel.len() < case.panel() {
                Outcome::Waiting
            } else if panel.iter().all(|x| x.split() == split) {
                publish(tx, case, id, false).await?
            } else {
                touch_status(tx, case.id, STATUS_OWNER_REVIEW).await?;
                audit(
                    &mut **tx,
                    SYSTEM_ACTOR,
                    "arbitration.disagreed",
                    case.id,
                    None,
                    &json!({ "rulings": panel.iter().map(|x| x.id).collect::<Vec<_>>() }),
                )
                .await?;
                alert_owners(tx, "owner_review", case.id, json!({})).await?;
                Outcome::Disagreed
            }
        }
        // 站长终裁与叫停后的改判都重新公示 72 小时（已用过申诉的不能再申诉，但仍公示满 72 小时才可执行）。
        RulingKind::Owner => publish(tx, case, id, false).await?,
        RulingKind::Appeal => publish(tx, case, id, true).await?,
    };
    Ok((id, outcome))
}

/// 公示 `ruling_id` 为生效裁决：`final_` 为真时直接可执行，否则开始 72 小时申诉期。通知双方并推送站长。
async fn publish(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    ruling_id: i64,
    final_: bool,
) -> Result<Outcome, sqlx::Error> {
    let now = OffsetDateTime::now_utc();
    let (status, appeal_until, ready_at) = if final_ {
        (STATUS_READY, case.appeal_until, Some(now))
    } else {
        (STATUS_APPEAL_WINDOW, Some(now + APPEAL_WINDOW), None)
    };
    sqlx::query!(
        "UPDATE arbitration_cases SET status = $2, final_ruling_id = $3, published_at = $4, appeal_until = $5,
                ready_at = $6, halted_from = NULL, updated_at = now()
         WHERE id = $1",
        case.id,
        status,
        ruling_id,
        now,
        appeal_until,
        ready_at
    )
    .execute(&mut **tx)
    .await?;
    audit(
        &mut **tx,
        SYSTEM_ACTOR,
        "arbitration.published",
        case.id,
        None,
        &json!({ "ruling_id": ruling_id, "status": status }),
    )
    .await?;
    for party in case.buyer_id.into_iter().chain(case.seller_id) {
        notify(&mut **tx, party, "published", case.id).await?;
    }
    let ruling = rulings(&mut **tx, case.id)
        .await?
        .into_iter()
        .find(|r| r.id == ruling_id);
    let evidence = evidence_links(&mut **tx, case.id).await?;
    alert_owners(
        tx,
        "published",
        case.id,
        json!({
            "status": status,
            "kind": ruling.as_ref().map(|r| r.kind.clone()),
            "to_buyer_bps": ruling.as_ref().map(|r| r.to_buyer_bps),
            "bond_to_buyer_bps": ruling.as_ref().map(|r| r.bond_to_buyer_bps),
            "reason": ruling.as_ref().map(|r| r.reason.clone()),
            "evidence": evidence,
        }),
    )
    .await?;
    Ok(if final_ {
        Outcome::Final
    } else {
        Outcome::Published
    })
}

/// 当事人申诉（调用方已确认在申诉期内、本案还没人申诉过）。
pub async fn appeal(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    user_id: i64,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE arbitration_cases SET status = 'appealed', appeal_by = $2, appeal_reason = $3, appealed_at = now(),
                updated_at = now()
         WHERE id = $1",
        case.id,
        user_id,
        reason
    )
    .execute(&mut **tx)
    .await?;
    audit(
        &mut **tx,
        user_id,
        "arbitration.appealed",
        case.id,
        Some(reason),
        &json!({}),
    )
    .await?;
    alert_owners(tx, "appealed", case.id, json!({ "reason": reason })).await
}

/// 站长在执行前叫停（调用方已确认状态允许）。
pub async fn halt(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    owner_id: i64,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE arbitration_cases SET status = 'halted', halted_from = status, halted_by = $2, halt_reason = $3,
                updated_at = now()
         WHERE id = $1",
        case.id,
        owner_id,
        reason
    )
    .execute(&mut **tx)
    .await?;
    audit(
        &mut **tx,
        owner_id,
        "arbitration.halted",
        case.id,
        Some(reason),
        &json!({ "from": case.status }),
    )
    .await?;
    alert_owners(tx, "halted", case.id, json!({ "reason": reason })).await
}

/// 站长解除叫停：回到叫停前的状态（申诉期已过的，worker 下一轮转为可执行）。
pub async fn resume(
    tx: &mut Transaction<'_, Postgres>,
    case: &Case,
    owner_id: i64,
) -> Result<(), sqlx::Error> {
    let to = case.halted_from.as_deref().unwrap_or(STATUS_READY);
    sqlx::query!(
        "UPDATE arbitration_cases SET status = $2, halted_from = NULL, updated_at = now() WHERE id = $1",
        case.id,
        to
    )
    .execute(&mut **tx)
    .await?;
    audit(
        &mut **tx,
        owner_id,
        "arbitration.resumed",
        case.id,
        None,
        &json!({ "to": to }),
    )
    .await
}

// ───────────────────────────── 裁决 Passkey ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct RulingPasskey {
    pub id: i64,
    pub credential_id: Vec<u8>,
    /// webauthn-rs 序列化的凭证（含签名计数）。
    pub credential: Value,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

pub async fn ruling_passkeys(
    db: impl PgExecutor<'_>,
    user_id: i64,
) -> Result<Vec<RulingPasskey>, sqlx::Error> {
    sqlx::query_as!(
        RulingPasskey,
        "SELECT id, credential_id, credential, created_at, last_used_at
         FROM passkeys WHERE user_id = $1 AND purpose = $2 ORDER BY id",
        user_id,
        PURPOSE_RULING
    )
    .fetch_all(db)
    .await
}

/// 保存裁决 Passkey；凭证 ID 已存在返回 `None`。
pub async fn insert_ruling_passkey(
    db: impl PgExecutor<'_>,
    user_id: i64,
    credential_id: &[u8],
    credential: &Value,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        "INSERT INTO passkeys (user_id, purpose, credential_id, credential, last_used_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (credential_id) DO NOTHING
         RETURNING id",
        user_id,
        PURPOSE_RULING,
        credential_id,
        credential
    )
    .fetch_optional(db)
    .await
}
