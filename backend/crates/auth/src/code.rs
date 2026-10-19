//! 邮箱验证码：6 位数字，10 分钟有效，60 秒内不能重发，最多试 5 次，用一次即失效。
//!
//! Valkey 里不存邮箱和验证码明文：键是邮箱的 HMAC，值是「邮箱 + 验证码」的 HMAC。

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use redis::Script;
use sha2::Sha256;

use forum_kv::{KvError, Pool};

pub const CODE_TTL: Duration = Duration::from_secs(600);
pub const RESEND_INTERVAL: Duration = Duration::from_secs(60);
pub const MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("resend too soon")]
    ResendTooSoon,
    #[error("invalid or expired code")]
    Invalid,
    #[error("too many attempts")]
    TooManyAttempts,
    #[error(transparent)]
    Kv(#[from] KvError),
}

impl From<redis::RedisError> for CodeError {
    fn from(e: redis::RedisError) -> Self {
        Self::Kv(e.into())
    }
}

/// 原子地记一次尝试并取出 MAC：键不存在返回 nil；否则返回 `{attempts, mac}`。
const ATTEMPT_SCRIPT: &str = r"
if redis.call('EXISTS', KEYS[1]) == 0 then
  return nil
end
local attempts = redis.call('HINCRBY', KEYS[1], 'attempts', 1)
return {attempts, redis.call('HGET', KEYS[1], 'mac')}
";

type HmacSha256 = Hmac<Sha256>;

/// 密钥太短（至少 32 字节）。
#[derive(Debug, thiserror::Error)]
#[error("auth secret must be at least 32 bytes")]
pub struct WeakSecret;

#[derive(Clone)]
pub struct CodeStore {
    pool: Pool,
    /// 已装入密钥的 HMAC 状态，每次使用时克隆。
    keyed: HmacSha256,
}

impl CodeStore {
    /// `secret` 来自 systemd-creds 加密配置。
    pub fn new(pool: Pool, secret: &[u8]) -> Result<Self, WeakSecret> {
        if secret.len() < 32 {
            return Err(WeakSecret);
        }
        let keyed = HmacSha256::new_from_slice(secret).map_err(|_| WeakSecret)?;
        Ok(Self { pool, keyed })
    }

    /// 各段带长度前缀再拼接，避免不同输入拼出同一串。
    fn keyed_with(&self, parts: &[&[u8]]) -> HmacSha256 {
        let mut mac = self.keyed.clone();
        for p in parts {
            mac.update(&(p.len() as u64).to_be_bytes());
            mac.update(p);
        }
        mac
    }

    fn mac(&self, parts: &[&[u8]]) -> Vec<u8> {
        self.keyed_with(parts).finalize().into_bytes().to_vec()
    }

    /// 邮箱的不可逆标识，用作限流等 Valkey 键，避免在 Valkey 里存邮箱明文。
    pub fn email_tag(&self, email: &str) -> String {
        hex::encode(self.mac(&[b"email", email.as_bytes()]))
    }

    fn key(&self, email: &str) -> String {
        format!("auth:code:{}", self.email_tag(email))
    }

    fn resend_key(&self, email: &str) -> String {
        format!("auth:resend:{}", self.email_tag(email))
    }

    /// 生成并保存新验证码，返回明文供发邮件。60 秒内重复调用返回 `ResendTooSoon`。
    pub async fn issue(&self, email: &str) -> Result<String, CodeError> {
        let mut conn = self.pool.get().await.map_err(KvError::from)?;
        let fresh: Option<String> = redis::cmd("SET")
            .arg(self.resend_key(email))
            .arg(1)
            .arg("NX")
            .arg("EX")
            .arg(RESEND_INTERVAL.as_secs())
            .query_async(&mut conn)
            .await?;
        if fresh.is_none() {
            return Err(CodeError::ResendTooSoon);
        }
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000u32));
        let mac = self.mac(&[b"code", email.as_bytes(), code.as_bytes()]);
        let key = self.key(email);
        redis::pipe()
            .atomic()
            .del(&key)
            .hset(&key, "mac", mac)
            .hset(&key, "attempts", 0)
            .expire(&key, i64::try_from(CODE_TTL.as_secs()).unwrap_or(600))
            .query_async::<()>(&mut conn)
            .await?;
        Ok(code)
    }

    /// 校验验证码；成功后立即作废。
    pub async fn verify(&self, email: &str, code: &str) -> Result<(), CodeError> {
        let key = self.key(email);
        let mut conn = self.pool.get().await.map_err(KvError::from)?;
        let found: Option<(u32, Vec<u8>)> = Script::new(ATTEMPT_SCRIPT)
            .key(&key)
            .invoke_async(&mut conn)
            .await?;
        let Some((attempts, stored)) = found else {
            return Err(CodeError::Invalid);
        };
        if attempts > MAX_ATTEMPTS {
            redis::cmd("DEL")
                .arg(&key)
                .query_async::<()>(&mut conn)
                .await?;
            return Err(CodeError::TooManyAttempts);
        }
        // verify_slice 是常量时间比较。
        let mac = self.keyed_with(&[b"code", email.as_bytes(), code.trim().as_bytes()]);
        if mac.verify_slice(&stored).is_err() {
            return Err(CodeError::Invalid);
        }
        redis::cmd("DEL")
            .arg(&key)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Option<CodeStore> {
        let Ok(url) = std::env::var("VALKEY_URL") else {
            eprintln!("VALKEY_URL not set, skipping valkey-backed test");
            return None;
        };
        Some(CodeStore::new(forum_kv::pool(&url).unwrap(), &[7u8; 32]).unwrap())
    }

    fn unique_email(tag: &str) -> String {
        let n: u64 = rand::rng().random();
        format!("{tag}-{n}@example.com")
    }

    #[tokio::test]
    async fn issue_verify_once() {
        let Some(s) = store() else { return };
        let email = unique_email("ok");
        let code = s.issue(&email).await.unwrap();
        assert_eq!(code.len(), 6);
        s.verify(&email, &code).await.unwrap();
        // 用过即失效
        assert!(matches!(
            s.verify(&email, &code).await,
            Err(CodeError::Invalid)
        ));
    }

    #[tokio::test]
    async fn resend_interval() {
        let Some(s) = store() else { return };
        let email = unique_email("resend");
        s.issue(&email).await.unwrap();
        assert!(matches!(
            s.issue(&email).await,
            Err(CodeError::ResendTooSoon)
        ));
    }

    #[tokio::test]
    async fn attempt_limit() {
        let Some(s) = store() else { return };
        let email = unique_email("limit");
        let code = s.issue(&email).await.unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..MAX_ATTEMPTS {
            assert!(matches!(
                s.verify(&email, wrong).await,
                Err(CodeError::Invalid)
            ));
        }
        // 第 6 次即便正确也拒绝，并作废验证码
        assert!(matches!(
            s.verify(&email, &code).await,
            Err(CodeError::TooManyAttempts)
        ));
        assert!(matches!(
            s.verify(&email, &code).await,
            Err(CodeError::Invalid)
        ));
    }
}
