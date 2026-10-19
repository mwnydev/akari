pub mod auth;
pub mod forums;
pub mod health;
pub mod me;
pub mod posts;
pub mod topics;

/// 会话里保存用户 ID 的键。
pub const SESSION_USER_ID: &str = "uid";
