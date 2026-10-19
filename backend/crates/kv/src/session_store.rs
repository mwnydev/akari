//! tower-sessions 的 Valkey 存储。
//!
//! 现有的 `tower-sessions-redis-store` 只支持 tower-sessions-core 0.14 且基于 fred，
//! 所以在 deadpool-redis 上自写：每个会话一个键 `sess:{id}`，值为 JSON，过期时间与会话一致。

use async_trait::async_trait;
use redis::AsyncCommands;
use redis::{ExistenceCheck, SetExpiry, SetOptions};
use tower_sessions_core::session::{Id, Record};
use tower_sessions_core::session_store::{self, Error, SessionStore};

use crate::Pool;

#[derive(Clone)]
pub struct ValkeySessionStore {
    pool: Pool,
}

impl std::fmt::Debug for ValkeySessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValkeySessionStore").finish_non_exhaustive()
    }
}

impl ValkeySessionStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    fn key(id: &Id) -> String {
        format!("sess:{id}")
    }

    /// 写入记录；`only_new` 为真时键已存在则返回 `false`（用于创建时避免 ID 冲突）。
    async fn write(&self, record: &Record, only_new: bool) -> session_store::Result<bool> {
        let value = serde_json::to_string(record).map_err(|e| Error::Encode(e.to_string()))?;
        let expire_at = u64::try_from(record.expiry_date.unix_timestamp())
            .map_err(|e| Error::Encode(e.to_string()))?;
        let mut opts = SetOptions::default().with_expiration(SetExpiry::EXAT(expire_at));
        if only_new {
            opts = opts.conditional_set(ExistenceCheck::NX);
        }
        let mut conn = self.pool.get().await.map_err(backend)?;
        let written: Option<String> = conn
            .set_options(Self::key(&record.id), value, opts)
            .await
            .map_err(backend)?;
        Ok(written.is_some())
    }
}

fn backend(e: impl std::fmt::Display) -> Error {
    Error::Backend(e.to_string())
}

#[async_trait]
impl SessionStore for ValkeySessionStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        // 128 位随机 ID 碰撞几乎不可能，但仍按约定处理：冲突就换一个。
        for _ in 0..8 {
            if self.write(record, true).await? {
                return Ok(());
            }
            record.id = Id::default();
        }
        Err(Error::Backend("session id collision".to_owned()))
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        self.write(record, false).await.map(|_| ())
    }

    async fn load(&self, id: &Id) -> session_store::Result<Option<Record>> {
        let mut conn = self.pool.get().await.map_err(backend)?;
        let value: Option<String> = conn.get(Self::key(id)).await.map_err(backend)?;
        value
            .map(|v| serde_json::from_str(&v).map_err(|e| Error::Decode(e.to_string())))
            .transpose()
    }

    async fn delete(&self, id: &Id) -> session_store::Result<()> {
        let mut conn = self.pool.get().await.map_err(backend)?;
        let _: i64 = conn.del(Self::key(id)).await.map_err(backend)?;
        Ok(())
    }
}
