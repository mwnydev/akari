//! 发件箱投递（邮件、站内通知与实时推送、管理员群 Telegram 告警）、浏览数回写、报价卡片过期、通知分区维护、
//! 发帖经验、每日信誉校对、图片转换、仲裁分配与期限、拍卖结算。
//!
//! 发件箱：轮询 `outbox`，`FOR UPDATE SKIP LOCKED` 认领一批（带租期，可多实例并行），逐条投递、逐条标记；
//! 失败按指数退避重试，重试不会成功或次数用尽的进死信并告警（[`drain_once`]）。
//! 发薪、超时处理等关键任务按 plan 第 2 节接入 apalis（PostgreSQL 队列）时，发件箱改为向 apalis 入队，这里只保留转发。

pub mod arbitration;
pub mod auctions;
pub mod exp;
mod failure;
pub mod images;
pub mod stats;
pub mod telegram;

use forum_core::chat::Event;
use forum_db::notifications::{self, NewNotification};
use forum_db::{PgPool, Postgres, Transaction, outbox};
use forum_kv::realtime::{Envelope, Route};
use forum_kv::{ForumCache, Realtime};
use forum_mail::{Email, Mailer, templates};
use serde::Deserialize;
use serde_json::{Value, json};

pub use crate::failure::Failure;
use crate::telegram::Telegram;

/// 每批最多认领条数。
pub const BATCH: usize = 10;
/// 单条消息的投递时限（发邮件、发 Telegram、写站内通知），超时算临时失败。一批在租期内一定处理完，
/// 正常情况下不会有消息因租期到了被别的 worker 重复投递。
pub const DELIVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const _: () = assert!(2 * BATCH as u64 * DELIVERY_TIMEOUT.as_secs() <= outbox::LEASE_SECS as u64);

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("unknown outbox kind")]
    UnknownKind,
    #[error("bad payload: {0}")]
    Payload(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct LoginCode {
    to: String,
    code: String,
}

#[derive(Deserialize)]
struct NewDevice {
    to: String,
    at: String,
}

#[derive(Deserialize)]
struct WithdrawAddressAdded {
    to: String,
    address: String,
    active_at: String,
    immediate: bool,
}

#[derive(Deserialize)]
struct StaffAlert {
    event: String,
    text: String,
}

/// 一条发件箱消息的投递方式。
#[derive(Debug)]
pub enum Delivery {
    Email(Email),
    /// 管理员 Telegram 群（纯文本）。
    Telegram(String),
    /// 站内：写通知（并推送）和 / 或推送新数据。
    InSite(InSite),
}

#[derive(Debug, Default)]
pub struct InSite {
    pub notifications: Vec<NewNotification>,
    pub push: Option<Push>,
    /// 合并的点赞通知：投递时统计这一楼（帖子 ID）上一条通知之后的新赞。
    pub like_digest: Option<i64>,
}

/// 推送已入库的数据（发布方在别的程序，只记了 ID 或状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Push {
    Message(i64),
    OrderMessage(i64),
    Order {
        order_id: i64,
        users: Vec<i64>,
        state: String,
        level: String,
        correction: bool,
    },
}

#[derive(Deserialize)]
struct To {
    to: i64,
    from: Option<i64>,
}

#[derive(Deserialize)]
struct Id {
    id: i64,
}

#[derive(Deserialize)]
struct OrderUpdated {
    id: i64,
    buyer_id: Option<i64>,
    seller_id: Option<i64>,
    state: String,
    level: String,
    cause: String,
    correction: bool,
}

/// 发给 `to` 的一条通知：内容为发件箱 payload 去掉 `to` / `from`，`extra` 合并进去。
fn notice(
    kind: &'static str,
    payload: &Value,
    extra: Option<(&str, &str)>,
) -> Result<InSite, RenderError> {
    let to: To = serde_json::from_value(payload.clone())?;
    let mut body = payload.clone();
    if let Some(m) = body.as_object_mut() {
        m.remove("to");
        m.remove("from");
        if let Some((k, v)) = extra {
            m.insert(k.to_owned(), Value::String(v.to_owned()));
        }
    }
    Ok(InSite {
        notifications: vec![NewNotification {
            user_id: to.to,
            kind,
            actor_id: to.from,
            payload: body,
        }],
        push: None,
        like_digest: None,
    })
}

#[derive(Deserialize)]
struct PostId {
    post_id: i64,
}

/// 订单状态变化：推给双方；链上事件与回滚另写通知（确认级别升级只推送，不通知）。
fn order_updated(payload: &Value) -> Result<InSite, RenderError> {
    let o: OrderUpdated = serde_json::from_value(payload.clone())?;
    let users: Vec<i64> = o.buyer_id.into_iter().chain(o.seller_id).collect();
    let notifications = if o.cause == "confirmation" {
        Vec::new()
    } else {
        users
            .iter()
            .map(|u| NewNotification {
                user_id: *u,
                kind: notifications::KIND_ORDER,
                actor_id: None,
                payload: json!({
                    "order_id": o.id, "state": o.state, "level": o.level, "correction": o.correction,
                }),
            })
            .collect()
    };
    Ok(InSite {
        notifications,
        push: Some(Push::Order {
            order_id: o.id,
            users,
            state: o.state,
            level: o.level,
            correction: o.correction,
        }),
        like_digest: None,
    })
}

/// 把发件箱消息渲染成投递动作。新增消息类型时在这里加分支。
///
/// 站内通知：`notify.reply` / `notify.mention` / `notify.comment` / `notify.message` / `notify.offer` /
/// `notify.auction` 各成同名通知；
/// `notify.likes` 在投递时合并成一条 `like` 通知（见 [`forum_db::floor::take_like_digest`]）；
/// `notify.system` 与其他 `notify.*`（如提币地址提醒）成为 `system` 通知（`event` = 去掉前缀的类型）。
pub fn render(msg: &outbox::Message) -> Result<Delivery, RenderError> {
    let p = &msg.payload;
    let in_site = match msg.kind.as_str() {
        "notify.reply" => notice(notifications::KIND_REPLY, p, None)?,
        "notify.mention" => notice(notifications::KIND_MENTION, p, None)?,
        "notify.comment" => notice(notifications::KIND_COMMENT, p, None)?,
        forum_db::floor::NOTIFY_LIKES => InSite {
            like_digest: Some(serde_json::from_value::<PostId>(p.clone())?.post_id),
            ..InSite::default()
        },
        "notify.message" => notice(notifications::KIND_MESSAGE, p, None)?,
        "notify.offer" => notice(notifications::KIND_OFFER, p, None)?,
        forum_db::auctions::NOTIFY => notice(notifications::KIND_AUCTION, p, None)?,
        "order.updated" => order_updated(p)?,
        "push.message" => InSite {
            push: Some(Push::Message(serde_json::from_value::<Id>(p.clone())?.id)),
            ..InSite::default()
        },
        "push.order_message" => InSite {
            push: Some(Push::OrderMessage(
                serde_json::from_value::<Id>(p.clone())?.id,
            )),
            ..InSite::default()
        },
        outbox::KIND_STAFF_ALERT => {
            let a: StaffAlert = serde_json::from_value(p.clone())?;
            return Ok(Delivery::Telegram(format!("[{}] {}", a.event, a.text)));
        }
        k if let Some(event) = k.strip_prefix("notify.") => {
            notice(notifications::KIND_SYSTEM, p, Some(("event", event)))?
        }
        _ => return render_email(msg).map(Delivery::Email),
    };
    Ok(Delivery::InSite(in_site))
}

fn render_email(msg: &outbox::Message) -> Result<Email, RenderError> {
    let key = format!("outbox-{}", msg.id);
    let (to, (subject, html, text)) = match msg.kind.as_str() {
        "email.login_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::login_code(&p.code))
        }
        "email.wallet_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::wallet_code(&p.code))
        }
        "email.admin_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::admin_code(&p.code))
        }
        "email.ruling_passkey_code" => {
            let p: LoginCode = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::ruling_passkey_code(&p.code))
        }
        forum_db::arbitration::EMAIL_ALERT => arbitration::alert_email(&msg.payload)?,
        "email.new_device" => {
            let p: NewDevice = serde_json::from_value(msg.payload.clone())?;
            (p.to, templates::new_device(&p.at))
        }
        "email.withdraw_address_added" => {
            let p: WithdrawAddressAdded = serde_json::from_value(msg.payload.clone())?;
            (
                p.to,
                templates::withdraw_address_added(&p.address, &p.active_at, p.immediate),
            )
        }
        _ => return Err(RenderError::UnknownKind),
    };
    Ok(Email {
        to,
        subject,
        html,
        text,
        idempotency_key: key,
    })
}

/// 认领一批到期消息并逐条投递，返回本批条数（0 表示暂时没有可发的）。
///
/// 每条消息各自成败：站内通知与「已发送」标记在这条消息自己的事务里写入，推送在提交之后发布（`push` 为空时不推送，
/// 如测试）；邮件与 Telegram 发出后再标记。一条失败只记它自己的失败（见 [`Failure`]），不影响同批的其他消息。
/// `telegram` 为空（未配置）时管理员群告警直接标记为已处理，不重试。
///
/// 重复投递：发出邮件 / Telegram 之后、标记之前出错（或租期内没处理完）的消息会再发一次。邮件带幂等键
/// `outbox-<ID>`（Resend 24 小时内只投递一次；Postmark 没有幂等键，可能重复）；Telegram 告警可能重复。
/// 站内通知与标记同一事务，不会重复。
pub async fn drain_once<M: Mailer>(
    db: &PgPool,
    mailer: &M,
    telegram: Option<&Telegram>,
    push: Option<&Realtime>,
) -> Result<usize, sqlx::Error> {
    let batch = outbox::claim(db, BATCH as i64, outbox::LEASE_SECS).await?;
    for msg in &batch {
        let result = if msg.attempts > outbox::MAX_ATTEMPTS {
            // 上次认领之后没有标记（worker 在投递中退出或崩溃），次数已经用尽
            Err(Failure::permanent(
                "abandoned: worker stopped during delivery",
            ))
        } else {
            tokio::time::timeout(DELIVERY_TIMEOUT, deliver_one(db, mailer, telegram, msg))
                .await
                .unwrap_or_else(|_| Err(Failure::transient("delivery timed out")))
        };
        match result {
            Ok(Some(publish)) => {
                if let Some(rt) = push {
                    for e in &publish {
                        if let Err(err) = rt.publish(e).await {
                            // 数据已在库里，客户端重连后会补拉。
                            tracing::warn!(error = %err, "realtime publish failed");
                        }
                    }
                }
            }
            Ok(None) => {
                tracing::warn!(id = msg.id, kind = %msg.kind, "outbox claim expired before delivery was recorded");
            }
            Err(failure) => record_failure(db, msg, failure).await,
        }
    }
    Ok(batch.len())
}

/// 投递一条消息并标记已发送，返回提交后要发布的推送；这次认领已失效（租期过后被重新认领，或已被站长处理）时
/// 返回 `None`，站内写入随之回滚。
async fn deliver_one<M: Mailer>(
    db: &PgPool,
    mailer: &M,
    telegram: Option<&Telegram>,
    msg: &outbox::Message,
) -> Result<Option<Vec<Envelope>>, Failure> {
    let mut publish = Vec::new();
    match render(msg)? {
        Delivery::Email(email) => mailer.send(&email).await?,
        Delivery::Telegram(text) => {
            if let Some(t) = telegram {
                t.send(&text).await.map_err(Failure::transient)?;
            }
        }
        Delivery::InSite(site) => {
            let mut tx = db.begin().await?;
            deliver(&mut tx, site, &mut publish).await?;
            if !outbox::mark_sent(&mut *tx, msg.id, msg.attempts).await? {
                return Ok(None);
            }
            tx.commit().await?;
            return Ok(Some(publish));
        }
    }
    Ok(outbox::mark_sent(db, msg.id, msg.attempts)
        .await?
        .then_some(publish))
}

/// 记一次失败；进了死信就推送管理员群（同一种消息一小时一条）。记录失败本身出错（数据库不可用）时只写日志：
/// 消息仍是认领状态，租期过后重新到期。
async fn record_failure(db: &PgPool, msg: &outbox::Message, failure: Failure) {
    let error = failure.error();
    // 只记 ID、类型和错误，不记收件人与内容。
    tracing::warn!(id = msg.id, kind = %msg.kind, attempts = msg.attempts, permanent = failure.permanent,
        error = %error, "outbox delivery failed");
    match outbox::mark_failed(db, msg.id, msg.attempts, &error, failure.permanent).await {
        Ok(Some(true)) => {
            if let Err(e) = dead_lettered(db, msg, &error).await {
                tracing::error!(id = msg.id, error = %e, "outbox dead-letter follow-up failed");
            }
        }
        Ok(Some(false)) => {}
        Ok(None) => {
            tracing::warn!(id = msg.id, kind = %msg.kind, "outbox claim expired before the failure was recorded");
        }
        Err(e) => {
            tracing::error!(id = msg.id, error = %e, "recording outbox failure failed; retried after the lease")
        }
    }
}

/// 管理员群里同一种消息的死信提醒间隔。
pub const DEAD_LETTER_ALERT_SECS: f64 = 3600.0;

/// 告警事件名（同时是去重键）：`outbox.dead_letter.<消息类型>`。
pub fn dead_letter_event(kind: &str) -> String {
    format!("outbox.dead_letter.{kind}")
}

async fn dead_lettered(db: &PgPool, msg: &outbox::Message, error: &str) -> Result<(), sqlx::Error> {
    tracing::error!(id = msg.id, kind = %msg.kind, attempts = msg.attempts, error = %error,
        "outbox message dead-lettered");
    if msg.kind == forum_db::floor::NOTIFY_LIKES
        && let Ok(p) = serde_json::from_value::<PostId>(msg.payload.clone())
    {
        forum_db::floor::release_like_digest(db, p.post_id).await?;
    }
    // 告警自己进了死信时不再告警（Telegram 不可用，告警也发不出去），只留日志
    if msg.kind != outbox::KIND_STAFF_ALERT {
        let text = format!(
            "发件箱消息 #{}（{}）尝试 {} 次后进了死信：{error}。一小时内同类型的死信不再单独提醒，\
             请在后台「发件箱」查看，修复后重新投递或丢弃。",
            msg.id, msg.kind, msg.attempts
        );
        outbox::staff_alert_deduped(
            db,
            &dead_letter_event(&msg.kind),
            &text,
            DEAD_LETTER_ALERT_SECS,
        )
        .await?;
    }
    Ok(())
}

/// 发件箱状况的巡检间隔：丢弃过期死信，按类型记录积压与死信条数。
pub const OUTBOX_REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
/// 最早一条待发消息等了这么久就算积压（写警告日志）。
const BACKLOG_SECS: i64 = 600;

/// 丢弃过期死信，并按类型记录待发、重试中、死信条数：一切正常只写一行汇总，有死信或积压的类型写警告。
/// 返回丢弃的死信条数。
pub async fn outbox_report(db: &PgPool) -> Result<usize, sqlx::Error> {
    let discarded = outbox::discard_expired(db, outbox::DEAD_LETTER_RETENTION_SECS).await?;
    let counts = outbox::counts(db).await?;
    let now = time::OffsetDateTime::now_utc();
    let (mut pending, mut dead) = (0, 0);
    for c in &counts {
        pending += c.pending;
        dead += c.dead;
        let waited = c.oldest_pending.map_or(0, |t| (now - t).whole_seconds());
        if c.dead > 0 || waited > BACKLOG_SECS {
            tracing::warn!(kind = %c.kind, pending = c.pending, retrying = c.retrying, dead = c.dead,
                oldest_pending_secs = waited, "outbox kind needs attention");
        }
    }
    tracing::info!(pending, dead, discarded, "outbox status");
    Ok(discarded as usize)
}

fn envelope(route: Route, event: &Event) -> Option<Envelope> {
    Envelope::new(route, event)
        .map_err(|e| tracing::error!(error = %e, "event encoding failed"))
        .ok()
}

/// 写通知并准备推送（在投递事务里读取要推送的行）。
async fn deliver(
    tx: &mut Transaction<'_, Postgres>,
    mut site: InSite,
    out: &mut Vec<Envelope>,
) -> Result<(), sqlx::Error> {
    if let Some(post_id) = site.like_digest
        && let Some(d) = forum_db::floor::take_like_digest(tx, post_id).await?
    {
        site.notifications.push(NewNotification {
            user_id: d.to,
            kind: notifications::KIND_LIKE,
            actor_id: d.likers.first().copied(),
            payload: json!({
                "topic_id": d.topic_id, "post_id": d.post_id, "floor": d.floor,
                "count": d.count, "likers": d.likers,
            }),
        });
    }
    for (user, n) in notifications::insert(&mut **tx, &site.notifications).await? {
        out.extend(envelope(
            Route::Users { ids: vec![user] },
            &Event::Notification { notification: n },
        ));
    }
    match site.push {
        None => {}
        Some(Push::Message(id)) => {
            if let Some(m) = forum_db::messages::by_id(&mut **tx, id).await?
                && let Some(c) =
                    forum_db::messages::conversation(&mut **tx, m.conversation_id).await?
            {
                out.extend(envelope(
                    Route::Users {
                        ids: c.members().to_vec(),
                    },
                    &Event::Message { message: m },
                ));
            }
        }
        Some(Push::OrderMessage(id)) => {
            if let Some(m) = forum_db::order_chat::by_id(&mut **tx, id).await?
                && let Some(p) = forum_db::orders::parties(&mut **tx, m.order_id).await?
            {
                out.extend(envelope(
                    Route::Order {
                        id: m.order_id,
                        users: p.users(),
                    },
                    &Event::OrderMessage { message: m },
                ));
            }
        }
        Some(Push::Order {
            order_id,
            users,
            state,
            level,
            correction,
        }) => out.extend(envelope(
            Route::Order {
                id: order_id,
                users,
            },
            &Event::Order {
                order_id,
                state,
                level,
                correction,
            },
        )),
    }
    Ok(())
}

/// 报价卡片过期检查间隔。卡片过期时间到了之后，接口读取时已按「已过期」展示，这里只是落库。
pub const OFFER_EXPIRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// 每批最多标记的过期卡片数。
pub const OFFER_EXPIRY_BATCH: i64 = 500;

/// 把到期的待确认报价卡片标为已过期（plan 第 7 节），返回总条数。一批满了就继续。
pub async fn expire_offers(db: &PgPool) -> Result<u64, sqlx::Error> {
    let mut total = 0;
    loop {
        let n = forum_db::trade::expire_due(db, OFFER_EXPIRY_BATCH).await?;
        total += n;
        if n < OFFER_EXPIRY_BATCH as u64 {
            return Ok(total);
        }
    }
}

/// 浏览数回写间隔（plan 第 8 节）。
pub const VIEW_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum FlushError {
    #[error(transparent)]
    Kv(#[from] forum_kv::KvError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// 把 Valkey 里累积的浏览数批量写入 `topics.view_count`，返回涉及的主题数。
/// 写库失败时把取走的计数加回 Valkey，下次再写。
pub async fn flush_views(db: &PgPool, cache: &ForumCache) -> Result<usize, FlushError> {
    let views = cache.take_views().await?;
    if views.is_empty() {
        return Ok(0);
    }
    let (ids, counts): (Vec<i64>, Vec<i64>) = views.iter().copied().unzip();
    if let Err(e) = forum_db::topics::add_views(db, &ids, &counts).await {
        cache.restore_views(&views).await?;
        return Err(e.into());
    }
    Ok(views.len())
}

/// 通知分区维护间隔（建未来两个月的分区、删 180 天前的分区）。
pub const PARTITION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(86_400);

/// 热门排名重算间隔（plan 第 9 节「热门榜刷新间隔 5 分钟」，见 [`forum_db::hot::refresh`]）。
pub const HOT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
