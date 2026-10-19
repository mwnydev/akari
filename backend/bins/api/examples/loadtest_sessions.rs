//! 压测用：给 `1..=N` 号用户直接在 Valkey 里建登录会话，输出对应的会话 Cookie（每行一个 `sid=…`），
//! 供 k6 的登录用户场景使用（deploy/loadtest/README）。不经过邮箱验证码登录，所以**只能用于压测库**。
//!
//!   VALKEY_URL=redis://127.0.0.1:6394 COOKIE_SECRET=<128 位十六进制> \
//!     cargo run --release -p forum-api --example loadtest_sessions -- 2000 > .run/loadtest-data/sessions.txt
//!
//! 会话格式与签名和 forum-api 完全相同（同一个 [`forum_kv::ValkeySessionStore`] 与 Cookie 签名密钥）。

use anyhow::{Context, bail};
use forum_api::SESSION_COOKIE;
use forum_api::routes::{SESSION_LOGIN_AT, SESSION_TOUCHED_AT, SESSION_USER_ID};
use time::{Duration, OffsetDateTime};
use tower_sessions::SessionStore;
use tower_sessions::cookie::{Cookie, CookieJar, Key};
use tower_sessions::session::{Id, Record};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let count: i64 = std::env::args()
        .nth(1)
        .context("usage: loadtest_sessions <users>")?
        .parse()?;
    let secret = hex::decode(std::env::var("COOKIE_SECRET").context("COOKIE_SECRET")?)?;
    if secret.len() != 64 {
        bail!("COOKIE_SECRET must be 64 bytes of hex");
    }
    if std::env::var("AKARI_ENV").is_ok_and(|e| e == "production") {
        bail!("refusing to mint sessions with AKARI_ENV=production");
    }
    let key = Key::from(&secret);
    let kv = forum_kv::pool(&std::env::var("VALKEY_URL").context("VALKEY_URL")?)?;
    let store = forum_kv::ValkeySessionStore::new(kv);
    let now = OffsetDateTime::now_utc();
    for uid in 1..=count {
        let mut record = Record {
            id: Id::default(),
            data: [
                (SESSION_USER_ID.to_owned(), uid.into()),
                (SESSION_LOGIN_AT.to_owned(), now.unix_timestamp().into()),
                (SESSION_TOUCHED_AT.to_owned(), now.unix_timestamp().into()),
            ]
            .into_iter()
            .collect(),
            expiry_date: now + Duration::days(1),
        };
        store.create(&mut record).await?;
        let mut jar = CookieJar::new();
        jar.signed_mut(&key)
            .add(Cookie::new(SESSION_COOKIE, record.id.to_string()));
        let cookie = jar.get(SESSION_COOKIE).context("signed cookie")?;
        println!("{}={}", SESSION_COOKIE, cookie.value());
    }
    Ok(())
}
