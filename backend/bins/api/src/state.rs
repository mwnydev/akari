use std::sync::Arc;

use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use forum_auth::{AnyCaptcha, CodeStore};
use forum_db::PgPool;
use forum_kv::{ForumCache, Pool};

use crate::admin::AdminState;
use crate::arbitration::ArbitrationState;
use crate::realtime::Hub;
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
    /// 站长后台：后台 Passkey 与后台来源（ADR 0012、0013）。
    pub admin: Arc<AdminState>,
    /// 论坛裁决 Passkey 与仲裁 Safe（ADR 0013）。
    pub arbitration: Arc<ArbitrationState>,
    /// WebSocket 连接表与实时推送（plan 第 7 节）。
    pub hub: Arc<Hub>,
    /// 图片存储（plan 第 12 节）；未配置时图片上传不可用，帖子里也不允许任何图片。
    pub media: Option<Arc<forum_media::Storage>>,
}

impl AppState {
    /// 帖子里允许的图片主机：只有本站图片的公开主机。
    pub fn image_hosts(&self) -> Vec<String> {
        self.media
            .iter()
            .map(|m| m.public_host().to_owned())
            .collect()
    }
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.device_key.clone()
    }
}
