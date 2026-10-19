use std::sync::Arc;

use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use forum_auth::{AnyCaptcha, CodeStore};
use forum_db::PgPool;
use forum_kv::{ForumCache, Pool};

use crate::wallet::WalletState;

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
    /// 板块列表、主题第 1 页、置顶列表、浏览数（plan 第 8 节）。
    pub cache: ForumCache,
    /// 本站域名：帖子里指向这些主机的绝对链接不算站外链接。
    pub internal_hosts: Arc<[String]>,
    /// 钱包 Passkey、钱包地址与报价（plan 第 5–7 节）。
    pub wallet: Arc<WalletState>,
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.device_key.clone()
    }
}
