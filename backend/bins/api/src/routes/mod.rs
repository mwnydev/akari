pub mod auth;
pub mod forums;
pub mod health;
pub mod me;
pub mod orders;
pub mod posts;
pub mod topics;
pub mod trade;
pub mod wallet;

/// 会话里保存用户 ID 的键。
pub const SESSION_USER_ID: &str = "uid";
