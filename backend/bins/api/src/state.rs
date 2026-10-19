use std::sync::Arc;

use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use forum_auth::{AnyCaptcha, CodeStore};
use forum_db::PgPool;
use forum_kv::Pool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub kv: Pool,
    pub codes: CodeStore,
    pub captcha: AnyCaptcha,
    /// 设备 Cookie 的签名密钥。
    pub device_key: Key,
    pub allowed_origins: Arc<[String]>,
    pub cookie_secure: bool,
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.device_key.clone()
    }
}
