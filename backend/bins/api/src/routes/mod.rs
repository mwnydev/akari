pub mod aa;
pub mod auth;
pub mod chain;
pub mod forums;
pub mod health;
pub mod me;
pub mod messages;
pub mod notifications;
pub mod order_chat;
pub mod orders;
pub mod posts;
pub mod presence;
pub mod topics;
pub mod trade;
pub mod uploads;
pub mod users;
pub mod wallet;
pub mod withdraw;
pub mod ws;

/// 会话里保存用户 ID 的键。
pub const SESSION_USER_ID: &str = "uid";
