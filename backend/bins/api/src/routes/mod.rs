pub mod aa;
pub mod admin;
pub mod admin_auth;
pub mod admin_forums;
pub mod admin_outbox;
pub mod arbitration;
pub mod auctions;
pub mod auth;
pub mod chain;
pub mod floor;
pub mod forums;
pub mod health;
pub mod hot;
pub mod me;
pub mod messages;
pub mod moderation;
pub mod notifications;
pub mod order_chat;
pub mod orders;
pub mod posts;
pub mod presence;
pub mod reports;
pub mod ruling_passkey;
pub mod search;
pub mod topics;
pub mod trade;
pub mod uploads;
pub mod users;
pub mod wallet;
pub mod withdraw;
pub mod ws;

/// 会话里保存用户 ID 的键。
pub const SESSION_USER_ID: &str = "uid";
/// 登录时间（Unix 秒）：会话最长有效期从这里算（[`crate::middleware::session_lifetime`]）。
pub const SESSION_LOGIN_AT: &str = "login_at";
/// 最近一次顺延闲置期限的时间（Unix 秒）。
pub const SESSION_TOUCHED_AT: &str = "touched_at";
