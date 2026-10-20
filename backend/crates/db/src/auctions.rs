//! 拍卖（plan 第 6、7 节「拍卖」，ADR 0011）：草稿 → 签授权上线 → 出价（只追加）→ 结束 → 成交证明 → 链上成交，
//! 以及违约、递补与流拍。规则（最低加价、防狙击、出价资格）见 [`forum_core::auction`]。
//!
//! - 写入方：forum-api（草稿、上线、出价、取消）、forum-worker（结束、违约、递补、流拍）、forum-sponsor（成交证明，
//!   只有它写 `auction_proofs`）、forum-indexer（链上成交与回滚，[`sync_order`]）。状态变化都是带条件的 UPDATE，
//!   并发下只会成功一次。
//! - 出价是一条语句（`queries/auction_bid.sql`）：锁住拍卖行、按库里的当前价校验最低出价、顺延结束时间、写出价记录，
//!   并发出价只有符合加价规则的会成功。
//! - 金额列是 `NUMERIC`：进出都用十进制字符串，单位是拍卖币种的最小单位（USDT 最小单位或分）。

use serde_json::{Value, json};
use sqlx::{PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;

use crate::{audit, outbox, sanctions};

pub const STATE_DRAFT: i16 = 0;
pub const STATE_LIVE: i16 = 1;
pub const STATE_AWAITING_PAYMENT: i16 = 2;
pub const STATE_SETTLED: i16 = 3;
pub const STATE_UNSOLD: i16 = 4;
pub const STATE_CANCELLED: i16 = 5;

/// 站内通知（发件箱 `notify.auction`，worker 投递成 `auction` 通知）的 `event`。
pub const NOTIFY: &str = "notify.auction";
pub const EVENT_OUTBID: &str = "outbid";
pub const EVENT_WON: &str = "won";
pub const EVENT_RUNNER_UP: &str = "runner_up";
pub const EVENT_SOLD: &str = "sold";
pub const EVENT_UNSOLD: &str = "unsold";
pub const EVENT_DEFAULTED: &str = "defaulted";
/// 定时提醒（[`remind_ending`]、[`remind_payment`]），每人每场（付款提醒每轮）只发一次。
pub const EVENT_ENDING_SOON: &str = "ending_soon";
pub const EVENT_PAY_REMINDER: &str = "pay_reminder";

/// 中标 / 递补与付款提醒的邮件（发件箱，worker 渲染）：付款期过了会记违约并禁拍，站内通知之外再发邮件。
pub const EMAIL_PAYMENT: &str = "email.auction_payment";

/// `auction_reminders.kind`
const REMINDER_ENDING: i16 = 1;
const REMINDER_PAYMENT: i16 = 2;

/// 系统操作的审计日志操作人（与仲裁相同）。
const SYSTEM_ACTOR: i64 = 0;

#[derive(Debug, Clone, PartialEq)]
pub struct Auction {
    pub id: i64,
    pub seller_id: i64,
    pub forum_id: i32,
    pub topic_id: Option<i64>,
    pub chain_id: i64,
    pub verifying_contract: Vec<u8>,
    pub seller: Vec<u8>,
    pub currency: i16,
    pub cny_per_usdt: i32,
    pub start_price: String,
    pub increment: String,
    pub reserve_price: String,
    pub duration_secs: i32,
    pub eligibility: i16,
    pub eligibility_value: Option<String>,
    pub terms: Value,
    pub content_hash: Vec<u8>,
    pub draft: Option<Value>,
    pub auth: Option<Value>,
    pub auth_hash: Option<Vec<u8>>,
    pub sign_deadline: Option<OffsetDateTime>,
    pub seller_signature: Option<Vec<u8>>,
    pub snipe_window_secs: Option<i32>,
    pub snipe_extension_secs: Option<i32>,
    pub extension_cap_secs: Option<i32>,
    pub state: i16,
    pub starts_at: Option<OffsetDateTime>,
    pub ends_at: Option<OffsetDateTime>,
    pub max_ends_at: Option<OffsetDateTime>,
    pub top_amount: Option<String>,
    pub top_bidder_id: Option<i64>,
    pub bid_count: i32,
    pub round: i16,
    pub pay_deadline: Option<OffsetDateTime>,
    pub order_id: Option<i64>,
    pub created_at: OffsetDateTime,
}

impl Auction {
    /// 当前价达到保留价（还没有出价时为假）。
    pub fn reserve_met(&self) -> bool {
        let num = |s: &str| s.parse::<u128>().ok();
        match (
            self.top_amount.as_deref().and_then(num),
            num(&self.reserve_price),
        ) {
            (Some(top), Some(reserve)) => top >= reserve,
            _ => false,
        }
    }
}

pub async fn by_id(db: impl PgExecutor<'_>, id: i64) -> Result<Option<Auction>, sqlx::Error> {
    sqlx::query_file_as!(Auction, "queries/auction_by_id.sql", id)
        .fetch_optional(db)
        .await
}

/// 拍卖帖主题对应的拍卖。
pub async fn id_by_topic(
    db: impl PgExecutor<'_>,
    topic_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!("SELECT id FROM auctions WHERE topic_id = $1", topic_id)
        .fetch_optional(db)
        .await
}

#[derive(Debug, Clone, Copy)]
pub struct NewAuction<'a> {
    pub seller_id: i64,
    pub forum_id: i32,
    pub chain_id: i64,
    pub verifying_contract: &'a [u8],
    pub seller: &'a [u8],
    pub currency: i16,
    pub cny_per_usdt: i32,
    pub start_price: &'a str,
    pub increment: &'a str,
    pub reserve_price: &'a str,
    pub duration_secs: i32,
    pub eligibility: i16,
    pub eligibility_value: Option<&'a str>,
    pub terms: &'a Value,
    pub content_hash: &'a [u8],
    pub draft: &'a Value,
}

pub async fn insert_draft(db: impl PgExecutor<'_>, a: NewAuction<'_>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"INSERT INTO auctions (seller_id, forum_id, chain_id, verifying_contract, seller, currency, cny_per_usdt,
                                 start_price, increment, reserve_price, duration_secs, eligibility, eligibility_value,
                                 terms, content_hash, draft)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::numeric, $9::text::numeric, $10::text::numeric, $11, $12,
                   $13::text::numeric, $14, $15, $16)
           RETURNING id"#,
        a.seller_id,
        a.forum_id,
        a.chain_id,
        a.verifying_contract,
        a.seller,
        a.currency,
        a.cny_per_usdt,
        a.start_price,
        a.increment,
        a.reserve_price,
        a.duration_secs,
        a.eligibility,
        a.eligibility_value,
        a.terms,
        a.content_hash,
        a.draft
    )
    .fetch_one(db)
    .await
}

/// 开始签名时生成的授权与这场拍卖的防狙击参数。
#[derive(Debug, Clone, Copy)]
pub struct SignStart<'a> {
    pub auth: &'a Value,
    pub auth_hash: &'a [u8],
    pub sign_deadline: OffsetDateTime,
    pub snipe_window_secs: i32,
    pub snipe_extension_secs: i32,
    pub extension_cap_secs: i32,
}

/// 草稿开始（或重新开始）签名：换上新的授权。只有草稿的卖家本人能做。
pub async fn start_signing(
    db: impl PgExecutor<'_>,
    id: i64,
    seller_id: i64,
    s: SignStart<'_>,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE auctions SET auth = $3, auth_hash = $4, sign_deadline = $5, snipe_window_secs = $6,
                snipe_extension_secs = $7, extension_cap_secs = $8, updated_at = now()
         WHERE id = $1 AND seller_id = $2 AND state = 0",
        id,
        seller_id,
        s.auth,
        s.auth_hash,
        s.sign_deadline,
        s.snipe_window_secs,
        s.snipe_extension_secs,
        s.extension_cap_secs
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 签名验证通过：挂上主题、记下签名、开始计时。`auth_hash` 必须仍是验证时的那份授权，且还在签名时限内。
pub async fn go_live(
    db: impl PgExecutor<'_>,
    id: i64,
    auth_hash: &[u8],
    signature: &[u8],
    topic_id: i64,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE auctions SET state = 1, seller_signature = $3, topic_id = $4, draft = NULL, starts_at = now(),
                ends_at = now() + make_interval(secs => duration_secs),
                max_ends_at = now() + make_interval(secs => duration_secs + extension_cap_secs),
                updated_at = now()
         WHERE id = $1 AND state = 0 AND auth_hash = $2 AND sign_deadline > now()",
        id,
        auth_hash,
        signature,
        topic_id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 卖家取消：草稿随时可以，进行中只能在第一口出价之前。
pub async fn cancel(db: impl PgExecutor<'_>, id: i64, seller_id: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE auctions SET state = 5, updated_at = now()
         WHERE id = $1 AND seller_id = $2 AND (state = 0 OR (state = 1 AND bid_count = 0 AND ends_at > now()))",
        id,
        seller_id
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

#[derive(Debug, Clone, Copy)]
pub struct NewBid<'a> {
    pub auction_id: i64,
    pub bidder_id: i64,
    pub amount: &'a str,
    pub amount_usdt: &'a str,
    /// 拍卖币种里 1 USDT 的最小单位数（最低加价的绝对下限，见 `Pricing::one_usdt`）。
    pub one_usdt: &'a str,
}

/// 出价成功后的拍卖状态。
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedBid {
    pub bid_id: i64,
    pub linked: bool,
    pub created_at: OffsetDateTime,
    pub ends_at: OffsetDateTime,
    pub bid_count: i32,
    pub reserve_met: bool,
    /// 被超过的上一位最高出价者（第一口出价时为空）。
    pub prev_bidder: Option<i64>,
}

/// 出价：一条语句里锁住拍卖、校验最低出价并顺延、写出价记录。不满足（已结束、出价太低、自己已是最高价）时返回空，
/// 由调用方重读拍卖给出原因。
pub async fn place_bid(
    db: impl PgExecutor<'_>,
    b: NewBid<'_>,
) -> Result<Option<PlacedBid>, sqlx::Error> {
    sqlx::query_file_as!(
        PlacedBid,
        "queries/auction_bid.sql",
        b.auction_id,
        b.bidder_id,
        b.amount,
        b.amount_usdt,
        b.one_usdt
    )
    .fetch_optional(db)
    .await
}

/// 两个账号是否关联（登录过同一台设备）。
pub async fn linked(db: impl PgExecutor<'_>, a: i64, b: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT users_linked($1, $2) AS "linked!""#, a, b)
        .fetch_one(db)
        .await
}

/// 公开出价记录的一行（出价人打码在接口层做）。
#[derive(Debug, Clone, PartialEq)]
pub struct Bid {
    pub id: i64,
    pub bidder_id: i64,
    pub username: Option<String>,
    pub amount: String,
    pub linked: bool,
    pub created_at: OffsetDateTime,
}

/// 出价记录，新的在前；`before` 为上一页最后一条的 ID。
pub async fn bids(
    db: impl PgExecutor<'_>,
    auction_id: i64,
    before: Option<i64>,
    limit: i64,
) -> Result<Vec<Bid>, sqlx::Error> {
    sqlx::query_file_as!(
        Bid,
        "queries/auction_bids_page.sql",
        auction_id,
        before.unwrap_or(i64::MAX),
        limit
    )
    .fetch_all(db)
    .await
}

/// 某人在这场拍卖里的最高出价（拍卖币种，十进制字符串）。
pub async fn best_bid_of(
    db: impl PgExecutor<'_>,
    auction_id: i64,
    bidder_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT amount::text AS "amount!" FROM auction_bids WHERE auction_id = $1 AND bidder_id = $2
           ORDER BY amount DESC LIMIT 1"#,
        auction_id,
        bidder_id
    )
    .fetch_optional(db)
    .await
}

/// 主题列表里拍卖帖的信息行。
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: i64,
    pub topic_id: i64,
    pub state: i16,
    pub currency: i16,
    pub cny_per_usdt: i32,
    /// 当前价：最高出价，还没有出价时为起拍价。
    pub current: String,
    pub bid_count: i32,
    pub ends_at: OffsetDateTime,
    pub reserve_met: bool,
}

pub async fn summaries(
    db: impl PgExecutor<'_>,
    topic_ids: &[i64],
) -> Result<Vec<Summary>, sqlx::Error> {
    if topic_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(Summary, "queries/auction_summaries.sql", topic_ids)
        .fetch_all(db)
        .await
}

// ───────────────────────────── 结算（forum-worker / forum-sponsor） ─────────────────────────────

/// 结束时间已到的进行中拍卖。
pub async fn due_ended(db: impl PgExecutor<'_>, limit: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM auctions WHERE state = 1 AND ends_at <= now() ORDER BY ends_at LIMIT $1",
        limit
    )
    .fetch_all(db)
    .await
}

/// 付款截止时间已过 `grace` 的待付款拍卖（宽限期让 indexer 先看到截止前的最后一笔付款）。
pub async fn due_payment(
    db: impl PgExecutor<'_>,
    grace: time::Duration,
    limit: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let before = OffsetDateTime::now_utc() - grace;
    sqlx::query_scalar!(
        "SELECT id FROM auctions WHERE state = 2 AND pay_deadline <= $1 ORDER BY pay_deadline LIMIT $2",
        before,
        limit
    )
    .fetch_all(db)
    .await
}

/// 下一位成交候选：出价达到保留价、还没拿过成交证明、没被跳过的出价人里出价最高的那口（同价先出的优先）。
/// forum-worker 用它挑人，forum-sponsor 签证明前用同一条查询独立核对，两边的「赢家」定义只有这一处。
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub bid_id: i64,
    pub bidder_id: i64,
    pub amount: String,
    pub amount_usdt: String,
}

pub async fn next_candidate(
    db: impl PgExecutor<'_>,
    auction_id: i64,
) -> Result<Option<Candidate>, sqlx::Error> {
    sqlx::query_file_as!(Candidate, "queries/auction_next_candidate.sql", auction_id)
        .fetch_optional(db)
        .await
}

/// 结束时不再符合出价资格的候选人：跳过，不记违约。
pub async fn pass(
    db: impl PgExecutor<'_>,
    auction_id: i64,
    bidder_id: i64,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO auction_passes (auction_id, bidder_id, reason) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        auction_id,
        bidder_id,
        reason
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 已签发的一份成交证明。
#[derive(Debug, Clone, PartialEq)]
pub struct Proof {
    pub auction_id: i64,
    pub round: i16,
    pub bid_id: i64,
    pub bidder_id: i64,
    pub auth_hash: Vec<u8>,
    pub winner: Vec<u8>,
    pub price: String,
    pub pay_deadline: OffsetDateTime,
    pub signature: Vec<u8>,
}

pub async fn proof(
    db: impl PgExecutor<'_>,
    auction_id: i64,
    round: i16,
) -> Result<Option<Proof>, sqlx::Error> {
    sqlx::query_as!(
        Proof,
        r#"SELECT auction_id, round, bid_id, bidder_id, auth_hash, winner, price::text AS "price!", pay_deadline,
                  signature
           FROM auction_proofs WHERE auction_id = $1 AND round = $2"#,
        auction_id,
        round
    )
    .fetch_optional(db)
    .await
}

/// 这份授权最新一轮的成交证明（forum-sponsor 代付 `createFromAuction` 前核对）。
pub async fn latest_proof_by_auth(
    db: impl PgExecutor<'_>,
    auth_hash: &[u8],
) -> Result<Option<Proof>, sqlx::Error> {
    sqlx::query_as!(
        Proof,
        r#"SELECT auction_id, round, bid_id, bidder_id, auth_hash, winner, price::text AS "price!", pay_deadline,
                  signature
           FROM auction_proofs WHERE auth_hash = $1 ORDER BY round DESC LIMIT 1"#,
        auth_hash
    )
    .fetch_optional(db)
    .await
}

/// 只有 forum-sponsor 调用。同一轮已有证明时不覆盖，返回是否写入。
pub async fn insert_proof(db: impl PgExecutor<'_>, p: &Proof) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "INSERT INTO auction_proofs (auction_id, round, bid_id, bidder_id, auth_hash, winner, price, pay_deadline,
                                     signature)
         VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric, $8, $9) ON CONFLICT DO NOTHING",
        p.auction_id,
        p.round,
        p.bid_id,
        p.bidder_id,
        p.auth_hash,
        p.winner,
        p.price,
        p.pay_deadline,
        p.signature
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 某一轮是否已记违约（forum-sponsor 发下一轮证明前核对）。
pub async fn has_default(
    db: impl PgExecutor<'_>,
    auction_id: i64,
    round: i16,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM auction_defaults WHERE auction_id = $1 AND round = $2) AS "e!""#,
        auction_id,
        round
    )
    .fetch_one(db)
    .await
}

/// 发出第 `round` 份证明后：待付款，付款截止时间取证明里的。只从上一轮推进，重复执行无效果。
pub async fn mark_offered(
    db: impl PgExecutor<'_>,
    id: i64,
    round: i16,
    pay_deadline: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE auctions SET state = 2, round = $2::smallint, pay_deadline = $3, updated_at = now()
         WHERE id = $1 AND state IN (1, 2) AND round = $2::smallint - 1",
        id,
        round,
        pay_deadline
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 流拍：从 `from` 状态（进行中已结束，或待付款且付款期已过）。
pub async fn mark_unsold(db: impl PgExecutor<'_>, id: i64, from: i16) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "UPDATE auctions SET state = 4, updated_at = now() WHERE id = $1 AND state = $2",
        id,
        from
    )
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 违约：记一次违约、自动禁拍 `ban_secs` 秒（全站），写审计日志与通知。同一轮重复执行不会重复记录。
/// 返回违约记录 ID（已记过时为空）。
pub async fn record_default(
    tx: &mut Transaction<'_, Postgres>,
    auction: &Auction,
    proof: &Proof,
    ban_secs: u32,
) -> Result<Option<i64>, sqlx::Error> {
    if has_default(&mut **tx, auction.id, proof.round).await? {
        return Ok(None);
    }
    let ends_at = OffsetDateTime::now_utc() + time::Duration::seconds(i64::from(ban_secs));
    let reason = format!("拍卖 #{} 中标后未在付款期内付款", auction.id);
    let sanction_id = sanctions::create(
        &mut **tx,
        sanctions::NewSanction {
            kind: sanctions::KIND_AUCTION_BAN,
            user_id: proof.bidder_id,
            forum_id: None,
            ends_at: Some(ends_at),
            reason: &reason,
            created_by: None,
        },
    )
    .await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO auction_defaults (auction_id, round, user_id, price, pay_deadline, sanction_id)
         VALUES ($1, $2, $3, $4::text::numeric, $5, $6) RETURNING id",
        auction.id,
        proof.round,
        proof.bidder_id,
        proof.price,
        proof.pay_deadline,
        sanction_id
    )
    .fetch_one(&mut **tx)
    .await?;
    audit::append(
        &mut **tx,
        SYSTEM_ACTOR,
        "auction.default",
        Some(&format!("user:{}", proof.bidder_id)),
        Some(&reason),
        &json!({ "auction_id": auction.id, "round": proof.round, "sanction_id": sanction_id }),
    )
    .await?;
    notify(
        &mut **tx,
        proof.bidder_id,
        auction,
        EVENT_DEFAULTED,
        json!({ "ban_until": ends_at.unix_timestamp() }),
    )
    .await?;
    Ok(Some(id))
}

/// 拍卖站内通知：发件箱 `notify.auction`，内容带上拍卖与主题 ID 和 `event`。
pub async fn notify(
    db: impl PgExecutor<'_>,
    to: i64,
    auction: &Auction,
    event: &str,
    extra: Value,
) -> Result<(), sqlx::Error> {
    enqueue_notice(db, to, auction.id, auction.topic_id, event, extra).await
}

async fn enqueue_notice(
    db: impl PgExecutor<'_>,
    to: i64,
    auction_id: i64,
    topic_id: Option<i64>,
    event: &str,
    extra: Value,
) -> Result<(), sqlx::Error> {
    let mut payload = json!({
        "to": to,
        "auction_id": auction_id,
        "topic_id": topic_id,
        "event": event,
    });
    if let (Some(p), Value::Object(extra)) = (payload.as_object_mut(), extra) {
        p.extend(extra);
    }
    outbox::enqueue(db, NOTIFY, &payload, outbox::PRIORITY_NORMAL).await?;
    Ok(())
}

/// 中标 / 递补（`reminder` 为假）或付款提醒的邮件，发到 `to` 的登录邮箱。只带拍卖编号与截止时间，不带帖子内容。
pub async fn payment_email(
    db: impl PgExecutor<'_>,
    to: i64,
    auction_id: i64,
    pay_deadline: OffsetDateTime,
    reminder: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO outbox (kind, payload, priority)
         SELECT $1, jsonb_build_object('to', email, 'auction_id', $3::bigint, 'pay_deadline', $4::bigint,
                                       'reminder', $5::boolean), $6
         FROM users WHERE id = $2",
        EMAIL_PAYMENT,
        to,
        auction_id,
        pay_deadline.unix_timestamp(),
        reminder,
        outbox::PRIORITY_NORMAL
    )
    .execute(db)
    .await?;
    Ok(())
}

/// 「即将结束」：进行中、`window` 内结束的拍卖，给每位出价人发一次（不含最后 `window` 里才第一次出价的人，
/// 他们正盯着这场）。防狙击顺延不会再发。一次最多 `limit` 人，返回发出的条数。
pub async fn remind_ending(
    tx: &mut Transaction<'_, Postgres>,
    window: time::Duration,
    limit: i64,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query_file!(
        "queries/auction_remind_ending.sql",
        window.as_seconds_f64(),
        limit,
        REMINDER_ENDING
    )
    .fetch_all(&mut **tx)
    .await?;
    for r in &rows {
        enqueue_notice(
            &mut **tx,
            r.user_id,
            r.auction_id,
            r.topic_id,
            EVENT_ENDING_SOON,
            json!({ "ends_at": r.ends_at.unix_timestamp() }),
        )
        .await?;
    }
    Ok(rows.len())
}

/// 付款提醒：待付款、`window` 内到付款截止时间的拍卖，给这一轮的中标者发站内通知与邮件，每轮一次。成交证明
/// 签发时离截止已不到 `window` 的不发（中标通知刚说过）。一次最多 `limit` 场，返回发出的条数。
pub async fn remind_payment(
    tx: &mut Transaction<'_, Postgres>,
    window: time::Duration,
    limit: i64,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query_file!(
        "queries/auction_remind_payment.sql",
        window.as_seconds_f64(),
        limit,
        REMINDER_PAYMENT
    )
    .fetch_all(&mut **tx)
    .await?;
    for r in &rows {
        let extra = json!({ "round": r.round, "pay_deadline": r.pay_deadline.unix_timestamp() });
        enqueue_notice(
            &mut **tx,
            r.user_id,
            r.auction_id,
            r.topic_id,
            EVENT_PAY_REMINDER,
            extra,
        )
        .await?;
        payment_email(&mut **tx, r.user_id, r.auction_id, r.pay_deadline, true).await?;
    }
    Ok(rows.len())
}

// ───────────────────────────── 链上成交（forum-indexer） ─────────────────────────────

/// 链上成交同步的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Synced {
    pub auction_id: i64,
    pub topic_id: Option<i64>,
    pub forum_id: i32,
    pub state: i16,
}

/// 订单是某场拍卖的成交（`AuctionSettled`，订单的 `offer_hash` 就是授权哈希）时，把拍卖标为已成交；
/// 订单被区块回滚（state 0）时退回待付款，之后照常按付款期处理。与订单折叠在同一事务里，回滚重放幂等。
pub async fn sync_order(
    tx: &mut Transaction<'_, Postgres>,
    order_id: i64,
) -> Result<Option<Synced>, sqlx::Error> {
    sqlx::query_as!(
        Synced,
        "UPDATE auctions a SET state = CASE WHEN o.state <> 0 THEN 3 ELSE 2 END,
                order_id = CASE WHEN o.state <> 0 THEN o.id END, updated_at = now()
         FROM escrow_orders o
         WHERE o.id = $1 AND o.auction_id IS NOT NULL AND a.auth_hash = o.offer_hash AND a.chain_id = o.chain_id
           AND a.verifying_contract = o.contract
           AND ((o.state <> 0 AND a.order_id IS DISTINCT FROM o.id) OR (o.state = 0 AND a.order_id = o.id))
         RETURNING a.id AS auction_id, a.topic_id, a.forum_id, a.state",
        order_id
    )
    .fetch_optional(&mut **tx)
    .await
}
