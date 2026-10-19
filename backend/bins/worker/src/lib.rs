//! 发件箱投递（邮件、站内通知与实时推送）、浏览数回写、报价卡片过期、通知分区维护。
//!
//! 现在是最小实现：轮询 `outbox`，`FOR UPDATE SKIP LOCKED` 锁一批、投递、标记，失败按指数退避重试。
//! 发薪、超时处理等关键任务按 plan 第 2 节接入 apalis（PostgreSQL 队列）时，发件箱改为向 apalis 入队，这里只保留转发。

use forum_core::chat::Event;
use forum_db::notifications::{self, NewNotification};
use forum_db::{PgPool, Postgres, Transaction, outbox};
use forum_kv::realtime::{Envelope, Route};
use forum_kv::{ForumCache, Realtime};
use forum_mail::{Email, Mailer, templates};
use serde::Deserialize;
use serde_json::{Value, json};

/// 每批最多处理条数。投递期间持有行锁，批不宜大。
pub const BATCH: usize = 10;

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

/// 一条发件箱消息的投递方式。
#[derive(Debug)]
pub enum Delivery {
    Email(Email),
    /// 站内：写通知（并推送）和 / 或推送新数据。
    InSite(InSite),
}

#[derive(Debug, Default)]
pub struct InSite {
    pub notifications: Vec<NewNotification>,
    pub push: Option<Push>,
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
    })
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
    })
}

/// 把发件箱消息渲染成投递动作。新增消息类型时在这里加分支。
///
/// 站内通知：`notify.reply` / `notify.mention` / `notify.message` / `notify.offer` 各成同名通知；
/// `notify.system` 与其他 `notify.*`（如提币地址提醒）成为 `system` 通知（`event` = 去掉前缀的类型）。
pub fn render(msg: &outbox::Message) -> Result<Delivery, RenderError> {
    let p = &msg.payload;
    let in_site = match msg.kind.as_str() {
        "notify.reply" => notice(notifications::KIND_REPLY, p, None)?,
        "notify.mention" => notice(notifications::KIND_MENTION, p, None)?,
        "notify.message" => notice(notifications::KIND_MESSAGE, p, None)?,
        "notify.offer" => notice(notifications::KIND_OFFER, p, None)?,
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

/// 处理一批到期消息，返回本批条数（0 表示暂时没有可发的）。
/// 站内通知与投递标记在同一事务里写入；推送在事务提交之后发布（`push` 为空时不推送，如测试）。
pub async fn drain_once<M: Mailer>(
    db: &PgPool,
    mailer: &M,
    push: Option<&Realtime>,
) -> Result<usize, sqlx::Error> {
    let mut tx = db.begin().await?;
    let batch = outbox::lock_due(&mut tx, BATCH as i64).await?;
    let mut publish = Vec::new();
    for msg in &batch {
        let result = match render(msg) {
            Ok(Delivery::Email(email)) => mailer.send(&email).await.map_err(|e| e.to_string()),
            Ok(Delivery::InSite(site)) => {
                deliver(&mut tx, site, &mut publish).await?;
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        };
        match result {
            Ok(()) => outbox::mark_sent(&mut *tx, msg.id).await?,
            Err(err) => {
                // 只记 ID、类型和错误，不记收件人与内容。
                tracing::warn!(id = msg.id, kind = %msg.kind, attempts = msg.attempts + 1, error = %err, "outbox delivery failed");
                outbox::mark_failed(&mut *tx, msg.id, &err).await?;
            }
        }
    }
    tx.commit().await?;
    if let Some(rt) = push {
        for e in &publish {
            if let Err(err) = rt.publish(e).await {
                // 数据已在库里，客户端重连后会补拉。
                tracing::warn!(error = %err, "realtime publish failed");
            }
        }
    }
    Ok(batch.len())
}

fn envelope(route: Route, event: &Event) -> Option<Envelope> {
    Envelope::new(route, event)
        .map_err(|e| tracing::error!(error = %e, "event encoding failed"))
        .ok()
}

/// 写通知并准备推送（在投递事务里读取要推送的行）。
async fn deliver(
    tx: &mut Transaction<'_, Postgres>,
    site: InSite,
    out: &mut Vec<Envelope>,
) -> Result<(), sqlx::Error> {
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
