//! sqlx 查询与迁移。所有 SQL 在编译期对照数据库检查（离线元数据在 `backend/.sqlx/`）。

pub mod audit;
pub mod chain;
pub mod credits;
pub mod exp;
pub mod forums;
pub mod jobs;
pub mod messages;
pub mod notifications;
pub mod order_chat;
pub mod orders;
pub mod outbox;
pub mod posts;
pub mod reputation;
pub mod sanctions;
pub mod settings;
pub mod sponsor;
pub mod staff;
pub mod topics;
pub mod trade;
pub mod uploads;
pub mod users;
pub mod wallet;
pub mod withdraw;

pub use sqlx::{PgPool, Postgres, Transaction};

/// 本 crate 的迁移，只增不改。
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// 连接池。`max` 按 plan 第 8 节：各程序连接池合计约 60。
pub async fn connect(url: &str, max: u32) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(url)
        .await
}
