//! 拍卖结算：流拍、中标、结束时再校验资格（跳过不符合的人）、违约记录与自动禁拍、次高者递补（最多 3 份证明）、
//! 链上已成交时不判违约、forum-sponsor 不可用时下一轮重试；即将结束与付款提醒（各发一次）、中标与付款提醒邮件。
//! forum-sponsor 与链上读取用假实现（真实的见 forum-api 的 `auction_e2e.rs`）。需要 PostgreSQL（`DATABASE_URL`）；
//! 游客缓存与名片缓存失效的用例另需 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use forum_core::Usdt;
use forum_db::PgPool;
use forum_db::auctions::{self as db, NewBid};
use forum_db::notifications::{self, NewNotification};
use forum_kv::ForumCache;
use forum_kv::guest_cache::{self, GuestCache, GuestTtl, Lookup};
use forum_worker::Delivery;
use forum_worker::auctions::{Config, Notify, Settlement, remind_once, run_once};
use serde_json::{Value, json};
use time::OffsetDateTime;

const U: u128 = 1_000_000;

/// 假的 forum-sponsor 与链：证明直接写库（与 forum-sponsor 写的同一张表），余额按钱包设置。
#[derive(Default)]
struct Fake {
    balances: Mutex<HashMap<Vec<u8>, u128>>,
    settled: AtomicBool,
    sponsor_down: AtomicBool,
    db: Mutex<Option<PgPool>>,
}

impl Fake {
    fn new(db: &PgPool) -> Self {
        let f = Self::default();
        *f.db.lock().unwrap() = Some(db.clone());
        f
    }
}

#[async_trait::async_trait]
impl Settlement for Fake {
    async fn usdt_balance(&self, wallet: &[u8]) -> anyhow::Result<Usdt> {
        Ok(Usdt(
            self.balances
                .lock()
                .unwrap()
                .get(wallet)
                .copied()
                .unwrap_or(u128::MAX),
        ))
    }

    async fn settled_on_chain(&self, _: &[u8], _: i64) -> anyhow::Result<bool> {
        Ok(self.settled.load(Ordering::SeqCst))
    }

    async fn issue_proof(
        &self,
        auction_id: i64,
        round: i16,
        bid_id: i64,
    ) -> anyhow::Result<OffsetDateTime> {
        anyhow::ensure!(!self.sponsor_down.load(Ordering::SeqCst), "sponsor down");
        let pool = self.db.lock().unwrap().clone().unwrap();
        let deadline = OffsetDateTime::now_utc() + time::Duration::hours(48);
        sqlx::query(
            "INSERT INTO auction_proofs (auction_id, round, bid_id, bidder_id, auth_hash, winner, price, pay_deadline, signature)
             SELECT $1, $2, b.id, b.bidder_id, a.auth_hash, w.address, b.amount, $4, '\\x01'
             FROM auction_bids b JOIN auctions a ON a.id = b.auction_id
             JOIN wallets w ON w.user_id = b.bidder_id
             WHERE b.id = $3",
        )
        .bind(auction_id)
        .bind(round)
        .bind(bid_id)
        .bind(deadline)
        .execute(&pool)
        .await?;
        Ok(deadline)
    }
}

async fn exec(db: &PgPool, sql: &'static str) {
    sqlx::raw_sql(sql).execute(db).await.unwrap();
}

/// 用户 1 是卖家，2–5 是出价人（各有钱包 0xa2… 等）。
async fn people(db: &PgPool) {
    exec(
        db,
        r"
        INSERT INTO users (email) VALUES ('s@x.test'), ('b2@x.test'), ('b3@x.test'), ('b4@x.test'), ('b5@x.test');
        INSERT INTO passkeys (user_id, purpose, credential_id, credential)
          SELECT id, 2, int8send(id), '{}' FROM users;
        INSERT INTO wallets (user_id, chain_id, address, passkey_id)
          SELECT u.id, 31337, decode(lpad(to_hex(160 + u.id), 40, '0'), 'hex'), p.id
          FROM users u JOIN passkeys p ON p.user_id = u.id;
        INSERT INTO forums (slug, name, allow_trade, allow_auction) VALUES ('market', 'market', true, true);
        ",
    )
    .await;
}

fn wallet(user: i64) -> Vec<u8> {
    let mut w = vec![0u8; 20];
    w[19] = u8::try_from(160 + user).unwrap();
    w
}

/// 上线中的 USDT 拍卖：起拍 10、幅度 1、保留价 `reserve` USDT，`eligibility` 为空表示不要求余额。
async fn auction(db: &PgPool, reserve: u128, eligibility: Option<(i16, &str)>) -> i64 {
    let (kind, value) = eligibility.map_or((0, None), |(k, v)| (k, Some(v)));
    let id = db::insert_draft(
        db,
        db::NewAuction {
            seller_id: 1,
            forum_id: 1,
            chain_id: 31_337,
            verifying_contract: &[0x44; 20],
            seller: &wallet(1),
            currency: 0,
            cny_per_usdt: 0,
            start_price: &(10 * U).to_string(),
            increment: &U.to_string(),
            reserve_price: &reserve.to_string(),
            duration_secs: 86_400,
            eligibility: kind,
            eligibility_value: value,
            terms: &json!({"ship_window": 86_400, "confirm_window": 86_400}),
            content_hash: &[7; 32],
            draft: &json!({}),
        },
    )
    .await
    .unwrap();
    let topic: i64 = sqlx::query_scalar(
        "INSERT INTO topics (forum_id, author_id, title, kind, last_post_at) VALUES (1, 1, 'auction', 3, now())
         RETURNING id",
    )
    .fetch_one(db)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE auctions SET state = 1, topic_id = $3, auth = '{}', auth_hash = $2, seller_signature = '\\x01',
                snipe_window_secs = 900, snipe_extension_secs = 900, extension_cap_secs = 86400,
                starts_at = now(), ends_at = now() + interval '1 hour', max_ends_at = now() + interval '25 hours'
         WHERE id = $1",
    )
    .bind(id)
    .bind({
        let mut h = vec![0u8; 32];
        h[0] = u8::try_from(id).unwrap();
        h
    })
    .bind(topic)
    .execute(db)
    .await
    .unwrap();
    id
}

async fn bid(db: &PgPool, auction_id: i64, bidder: i64, usdt: u128) {
    let s = (usdt * U).to_string();
    db::place_bid(
        db,
        NewBid {
            auction_id,
            bidder_id: bidder,
            amount: &s,
            amount_usdt: &s,
            one_usdt: &U.to_string(),
        },
    )
    .await
    .unwrap()
    .expect("bid accepted");
}

async fn end(db: &PgPool, id: i64) {
    sqlx::query("UPDATE auctions SET ends_at = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(db)
        .await
        .unwrap();
}

/// 当前一份证明的付款截止时间已过（宽限期设为 0）。
async fn overdue(db: &PgPool, id: i64) {
    sqlx::query(
        "UPDATE auction_proofs SET pay_deadline = now() - interval '1 second'
         WHERE auction_id = $1 AND round = (SELECT round FROM auctions WHERE id = $1)",
    )
    .bind(id)
    .execute(db)
    .await
    .unwrap();
    sqlx::query("UPDATE auctions SET pay_deadline = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(db)
        .await
        .unwrap();
}

const NO_GRACE: Config = Config {
    grace: time::Duration::ZERO,
};

async fn state(db: &PgPool, id: i64) -> (i16, i16) {
    sqlx::query_as("SELECT state, round FROM auctions WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

/// `(round, bidder)` 按轮次。
async fn proofs(db: &PgPool, id: i64) -> Vec<(i16, i64)> {
    sqlx::query_as(
        "SELECT round, bidder_id FROM auction_proofs WHERE auction_id = $1 ORDER BY round",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .unwrap()
}

/// 发给 `to` 的拍卖通知的 `event`，按时间。
async fn events(db: &PgPool, to: i64) -> Vec<String> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'notify.auction' AND (payload->>'to')::bigint = $1 ORDER BY id",
    )
    .bind(to)
    .fetch_all(db)
    .await
    .unwrap();
    rows.iter()
        .map(|p| p["event"].as_str().unwrap().to_owned())
        .filter(|e| e != "outbid")
        .collect()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn below_reserve_or_no_bids_is_unsold(db: PgPool) {
    people(&db).await;
    let fake = Fake::new(&db);
    let none = auction(&db, 10 * U, None).await;
    let low = auction(&db, 30 * U, None).await;
    bid(&db, low, 2, 20).await;
    // 还没到结束时间：不动
    assert_eq!(run_once(&db, &fake, NO_GRACE, None).await.unwrap(), 0);
    end(&db, none).await;
    end(&db, low).await;
    assert_eq!(run_once(&db, &fake, NO_GRACE, None).await.unwrap(), 2);
    assert_eq!(state(&db, none).await, (db::STATE_UNSOLD, 0));
    assert_eq!(state(&db, low).await, (db::STATE_UNSOLD, 0));
    assert!(proofs(&db, low).await.is_empty());
    assert_eq!(events(&db, 1).await, ["unsold", "unsold"]);
    // 再跑一轮没有变化
    assert_eq!(run_once(&db, &fake, NO_GRACE, None).await.unwrap(), 0);
}

/// 状态变化（这里是流拍）让游客缓存里带这条拍卖信息行的板块列表失效。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn state_changes_invalidate_guest_lists(db: PgPool) {
    let Some(url) = forum_testkit::valkey_url() else {
        return;
    };
    people(&db).await;
    let fake = Fake::new(&db);
    let id = auction(&db, 10 * U, None).await;
    let kv = forum_kv::pool(&url).unwrap();
    let ns = format!(
        "wauction{}{}:",
        std::process::id(),
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let guest = GuestCache::new(kv.clone(), ns.as_str());
    let versions = [guest_cache::forums(), guest_cache::board(1)];
    let ttl = GuestTtl {
        fresh: std::time::Duration::from_secs(30),
        stale: std::time::Duration::from_secs(30),
    };
    let Lookup::Build(ticket) = guest.lookup("board:1:1", &versions).await.unwrap() else {
        panic!("empty cache must ask to build");
    };
    guest.store("board:1:1", ticket, "{}", ttl).await.unwrap();
    let hit = guest.lookup("board:1:1", &versions).await.unwrap();
    assert!(matches!(hit, Lookup::Hit(_)), "{hit:?}");

    end(&db, id).await;
    let notify = Notify {
        realtime: forum_kv::Realtime::new(kv.clone(), ns.as_str()),
        guest: guest.clone(),
        cards: ForumCache::new(kv, ns.as_str()),
    };
    assert_eq!(
        run_once(&db, &fake, NO_GRACE, Some(&notify)).await.unwrap(),
        1
    );
    assert_eq!(state(&db, id).await, (db::STATE_UNSOLD, 0));
    let after = guest.lookup("board:1:1", &versions).await.unwrap();
    assert!(matches!(after, Lookup::Build(_)), "{after:?}");
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn winner_is_the_highest_eligible_bidder(db: PgPool) {
    people(&db).await;
    let fake = Fake::new(&db);
    // 余额 ≥ 出价的 100%
    let id = auction(&db, 15 * U, Some((1, "100"))).await;
    bid(&db, id, 2, 15).await;
    bid(&db, id, 3, 20).await;
    bid(&db, id, 4, 25).await;
    // 最高价 4 号余额不够、3 号被禁拍：都跳过（不算违约），2 号中标。
    fake.balances.lock().unwrap().insert(wallet(4), 25 * U - 1);
    exec(
        &db,
        "INSERT INTO sanctions (kind, user_id, ends_at, reason, created_by) VALUES (2, 3, now() + interval '1 day', 'x', 1)",
    )
    .await;
    end(&db, id).await;
    // forum-sponsor 不可用：这一轮不变，下一轮重试
    fake.sponsor_down.store(true, Ordering::SeqCst);
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(state(&db, id).await, (db::STATE_LIVE, 0));
    fake.sponsor_down.store(false, Ordering::SeqCst);
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(state(&db, id).await, (db::STATE_AWAITING_PAYMENT, 1));
    assert_eq!(proofs(&db, id).await, [(1, 2)]);
    let passes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT bidder_id, reason FROM auction_passes WHERE auction_id = $1 ORDER BY bidder_id",
    )
    .bind(id)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(passes, [(3, "banned".into()), (4, "balance".into())]);
    assert_eq!(events(&db, 2).await, ["won"]);
    assert_eq!(events(&db, 1).await, ["sold"]);
    let deadline: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT pay_deadline FROM auctions WHERE id = $1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(deadline.unwrap() > OffsetDateTime::now_utc() + time::Duration::hours(47));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn defaults_ban_the_winner_and_pass_to_runners_up(db: PgPool) {
    people(&db).await;
    let fake = Fake::new(&db);
    let id = auction(&db, 10 * U, None).await;
    for (bidder, usdt) in [(2, 10), (3, 11), (4, 12), (5, 13)] {
        bid(&db, id, bidder, usdt).await;
    }
    end(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(proofs(&db, id).await, [(1, 5)]);

    // 付款期内什么也不做
    assert_eq!(run_once(&db, &fake, NO_GRACE, None).await.unwrap(), 0);
    // 付款期过了、但宽限期内：不判违约
    overdue(&db, id).await;
    let grace = Config {
        grace: time::Duration::minutes(5),
    };
    assert_eq!(run_once(&db, &fake, grace, None).await.unwrap(), 0);
    // 链上其实已成交（indexer 没跟上）：不判违约，等 indexer
    fake.settled.store(true, Ordering::SeqCst);
    assert_eq!(run_once(&db, &fake, NO_GRACE, None).await.unwrap(), 0);
    fake.settled.store(false, Ordering::SeqCst);

    // 违约：记录、全站禁拍 30 天（系统处罚，没有操作人），次高者 4 号递补
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(state(&db, id).await, (db::STATE_AWAITING_PAYMENT, 2));
    assert_eq!(proofs(&db, id).await, [(1, 5), (2, 4)]);
    let (user, sanction): (i64, i64) = sqlx::query_as(
        "SELECT user_id, sanction_id FROM auction_defaults WHERE auction_id = $1 AND round = 1",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(user, 5);
    let s = forum_db::sanctions::by_id(&db, sanction)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.kind, forum_db::sanctions::KIND_AUCTION_BAN);
    assert_eq!((s.user_id, s.forum_id, s.created_by), (Some(5), None, None));
    let days = (s.ends_at.unwrap() - OffsetDateTime::now_utc()).whole_days();
    assert!((29..=30).contains(&days), "{days}");
    assert_eq!(
        forum_db::sanctions::active_kind(&db, 5, &[forum_db::sanctions::KIND_AUCTION_BAN], Some(1))
            .await
            .unwrap(),
        Some(forum_db::sanctions::KIND_AUCTION_BAN)
    );
    assert_eq!(events(&db, 5).await, ["won", "defaulted"]);
    assert_eq!(events(&db, 4).await, ["runner_up"]);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM admin_audit_logs WHERE action = 'auction.default' AND actor_id = 0",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(audit, 1);

    // 递补也违约：再递补一次（第 3 份证明给 3 号）；第三位也违约 → 流拍，2 号拿不到证明。
    overdue(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(proofs(&db, id).await, [(1, 5), (2, 4), (3, 3)]);
    overdue(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(state(&db, id).await, (db::STATE_UNSOLD, 3));
    assert_eq!(proofs(&db, id).await.len(), 3);
    let defaults: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auction_defaults WHERE auction_id = $1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(defaults, 3);
    assert!(events(&db, 2).await.is_empty());
    assert_eq!(events(&db, 1).await, ["sold", "sold", "sold", "unsold"]);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn runner_up_must_reach_the_reserve(db: PgPool) {
    people(&db).await;
    let fake = Fake::new(&db);
    let id = auction(&db, 20 * U, None).await;
    bid(&db, id, 2, 15).await;
    bid(&db, id, 3, 25).await;
    end(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(proofs(&db, id).await, [(1, 3)]);
    overdue(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    // 2 号只出到 15，低于保留价 20：没人可递补，流拍
    assert_eq!(state(&db, id).await, (db::STATE_UNSOLD, 1));
    assert_eq!(proofs(&db, id).await.len(), 1);
}

/// 发件箱里的拍卖付款邮件 `(收件人, 是否提醒)`，按时间。
async fn payment_emails(db: &PgPool) -> Vec<(String, bool)> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'email.auction_payment' ORDER BY id",
    )
    .fetch_all(db)
    .await
    .unwrap();
    rows.iter()
        .map(|p| {
            (
                p["to"].as_str().unwrap().to_owned(),
                p["reminder"].as_bool().unwrap(),
            )
        })
        .collect()
}

/// 结束时间改到 `mins` 分钟后（模拟防狙击顺延或时间流逝）。
async fn ends_in(db: &PgPool, id: i64, mins: i64) {
    sqlx::query(
        "UPDATE auctions SET ends_at = now() + make_interval(mins => $2::int) WHERE id = $1",
    )
    .bind(id)
    .bind(i32::try_from(mins).unwrap())
    .execute(db)
    .await
    .unwrap();
}

/// 这场的出价都提前 `hours` 小时（出价在最后一小时之前）。
async fn bids_earlier(db: &PgPool, id: i64, hours: i32) {
    sqlx::query(
        "UPDATE auction_bids SET created_at = created_at - make_interval(hours => $2) WHERE auction_id = $1",
    )
    .bind(id)
    .bind(hours)
    .execute(db)
    .await
    .unwrap();
}

/// 当前一份证明 `hours` 小时后到付款截止；`issued_hours_ago` 是证明签发了多久。
async fn pay_due_in(db: &PgPool, id: i64, hours: i32, issued_hours_ago: i32) {
    sqlx::query(
        "UPDATE auction_proofs SET pay_deadline = now() + make_interval(hours => $2),
                created_at = now() - make_interval(hours => $3)
         WHERE auction_id = $1 AND round = (SELECT round FROM auctions WHERE id = $1)",
    )
    .bind(id)
    .bind(hours)
    .bind(issued_hours_ago)
    .execute(db)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE auctions SET pay_deadline = now() + make_interval(hours => $2) WHERE id = $1",
    )
    .bind(id)
    .bind(hours)
    .execute(db)
    .await
    .unwrap();
}

/// 渲染发件箱里的一条消息（worker 投递时用的同一个函数）。
fn render(id: i64, kind: &str, payload: Value) -> Delivery {
    forum_worker::render(&forum_db::outbox::Message {
        id,
        kind: kind.to_owned(),
        payload,
        attempts: 0,
    })
    .unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn ending_soon_reminds_each_earlier_bidder_once(db: PgPool) {
    people(&db).await;
    let id = auction(&db, 10 * U, None).await;
    ends_in(&db, id, 180).await;
    bid(&db, id, 2, 10).await;
    bid(&db, id, 3, 11).await;
    bid(&db, id, 2, 12).await;
    bids_earlier(&db, id, 2).await;
    // 还剩 3 小时：不提醒
    assert_eq!(remind_once(&db).await.unwrap(), 0);

    ends_in(&db, id, 30).await;
    // 最后一小时里才第一次出价的 4 号不提醒
    bid(&db, id, 4, 13).await;
    assert_eq!(remind_once(&db).await.unwrap(), 2);
    assert_eq!(events(&db, 2).await, ["ending_soon"]);
    assert_eq!(events(&db, 3).await, ["ending_soon"]);
    assert!(events(&db, 4).await.is_empty());
    assert!(
        events(&db, 1).await.is_empty(),
        "the seller is not a bidder"
    );
    // 提醒只给本人：内容里没有其他出价人
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'notify.auction' AND payload->>'event' = 'ending_soon'
         ORDER BY id LIMIT 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let mut keys: Vec<&str> = payload
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["auction_id", "ends_at", "event", "to", "topic_id"]);

    // 再巡检、顺延回 1 小时以上再回落：都不再提醒
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    ends_in(&db, id, 120).await;
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    ends_in(&db, id, 20).await;
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    assert_eq!(events(&db, 2).await, ["ending_soon"]);

    // 已结束的拍卖不提醒
    let done = auction(&db, 10 * U, None).await;
    bid(&db, done, 5, 10).await;
    bids_earlier(&db, done, 2).await;
    end(&db, done).await;
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    assert!(events(&db, 5).await.is_empty());
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn winners_are_emailed_and_reminded_before_the_payment_deadline(db: PgPool) {
    people(&db).await;
    let fake = Fake::new(&db);
    let id = auction(&db, 10 * U, None).await;
    bid(&db, id, 2, 10).await;
    bid(&db, id, 3, 11).await;
    end(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    // 中标当时：站内通知（带付款截止时间）+ 邮件
    assert_eq!(events(&db, 3).await, ["won"]);
    assert_eq!(payment_emails(&db).await, [("b3@x.test".into(), false)]);

    // 离截止还有 48 小时、7 小时：不提醒
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    pay_due_in(&db, id, 7, 41).await;
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    // 不到 6 小时：提醒一次（站内 + 邮件）
    pay_due_in(&db, id, 5, 43).await;
    assert_eq!(remind_once(&db).await.unwrap(), 1);
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    assert_eq!(events(&db, 3).await, ["won", "pay_reminder"]);
    assert_eq!(
        payment_emails(&db).await,
        [("b3@x.test".into(), false), ("b3@x.test".into(), true)]
    );
    assert!(events(&db, 2).await.is_empty());

    // 违约 → 递补者 2 号：递补当时的邮件，这一轮另提醒一次
    overdue(&db, id).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    assert_eq!(state(&db, id).await, (db::STATE_AWAITING_PAYMENT, 2));
    assert_eq!(events(&db, 2).await, ["runner_up"]);
    pay_due_in(&db, id, 5, 19).await;
    assert_eq!(remind_once(&db).await.unwrap(), 1);
    assert_eq!(events(&db, 2).await, ["runner_up", "pay_reminder"]);
    let reminder: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'notify.auction' AND payload->>'event' = 'pay_reminder'
         ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(reminder["round"], 2);
    assert_eq!(
        payment_emails(&db).await[2..],
        [("b2@x.test".into(), false), ("b2@x.test".into(), true)]
    );

    // 证明签发时离截止已不到 6 小时：中标通知刚说过，不再提醒
    let late = auction(&db, 10 * U, None).await;
    bid(&db, late, 4, 10).await;
    end(&db, late).await;
    run_once(&db, &fake, NO_GRACE, None).await.unwrap();
    pay_due_in(&db, late, 5, 0).await;
    assert_eq!(remind_once(&db).await.unwrap(), 0);
    assert_eq!(events(&db, 4).await, ["won"]);
}

/// 新的通知内容能投递（`auction` 类型过得了通知类型约束），付款邮件能渲染。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn reminder_messages_render_and_deliver(db: PgPool) {
    people(&db).await;
    let id = auction(&db, 10 * U, None).await;
    bid(&db, id, 2, 10).await;
    bids_earlier(&db, id, 2).await;
    ends_in(&db, id, 30).await;
    remind_once(&db).await.unwrap();
    let rows: Vec<(i64, String, Value)> =
        sqlx::query_as("SELECT id, kind, payload FROM outbox ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    let mut items: Vec<NewNotification> = Vec::new();
    for (id, kind, payload) in rows {
        let Delivery::InSite(site) = render(id, &kind, payload) else {
            panic!("{kind} must be delivered in-site");
        };
        items.extend(site.notifications);
    }
    for event in [db::EVENT_PAY_REMINDER, db::EVENT_WON, db::EVENT_RUNNER_UP] {
        let Delivery::InSite(site) = render(
            0,
            db::NOTIFY,
            json!({"to": 3, "auction_id": id, "topic_id": 1, "event": event, "round": 1, "pay_deadline": 1}),
        ) else {
            panic!("in-site");
        };
        items.extend(site.notifications);
    }
    assert_eq!(items.len(), 4);
    let inserted = notifications::insert(&db, &items).await.unwrap();
    assert_eq!(inserted.len(), 4);
    assert!(
        inserted
            .iter()
            .all(|(_, n)| n.kind == notifications::KIND_AUCTION)
    );
    assert_eq!(inserted[0].0, 2);
    assert_eq!(inserted[0].1.payload["event"], db::EVENT_ENDING_SOON);

    let Delivery::Email(mail) = render(
        9,
        db::EMAIL_PAYMENT,
        json!({"to": "b3@x.test", "auction_id": id, "pay_deadline": 1_792_000_000, "reminder": true}),
    ) else {
        panic!("payment reminders are emailed");
    };
    assert_eq!(mail.to, "b3@x.test");
    assert!(mail.text.contains("2026-10-14 17:46 UTC"), "{}", mail.text);
}

/// 违约记录公开显示在名片上：记违约后名片缓存失效。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn defaults_invalidate_the_card_cache(db: PgPool) {
    let Some(url) = forum_testkit::valkey_url() else {
        return;
    };
    people(&db).await;
    let fake = Fake::new(&db);
    let kv = forum_kv::pool(&url).unwrap();
    let ns = format!(
        "wcards{}{}:",
        std::process::id(),
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let notify = Notify {
        realtime: forum_kv::Realtime::new(kv.clone(), ns.as_str()),
        guest: GuestCache::new(kv.clone(), ns.as_str()),
        cards: ForumCache::new(kv, ns.as_str()),
    };
    let id = auction(&db, 10 * U, None).await;
    bid(&db, id, 2, 10).await;
    end(&db, id).await;
    run_once(&db, &fake, NO_GRACE, Some(&notify)).await.unwrap();
    notify
        .cards
        .cards_put(&[
            (2, vec![("l", "1".to_owned())]),
            (3, vec![("l", "1".to_owned())]),
        ])
        .await
        .unwrap();
    overdue(&db, id).await;
    run_once(&db, &fake, NO_GRACE, Some(&notify)).await.unwrap();
    assert_eq!(state(&db, id).await, (db::STATE_UNSOLD, 1));
    let cached = notify.cards.cards_get(&[2, 3]).await.unwrap();
    assert!(cached[0].is_none(), "the defaulter's card is refetched");
    assert!(cached[1].is_some(), "other cards stay cached");
}
