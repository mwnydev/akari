//! WebAuthn 仪式状态（challenge）的一次性存取（plan 第 5 节）：存 Valkey 5 分钟，取出即删除，不能重放。
//! 钱包、后台与论坛裁决 Passkey 共用；`kind` 带用途前缀（`wallet:reg`、`admin:login`、`arbitration:auth` 等），
//! 各自的 challenge 互不覆盖。

use std::time::Duration;

use forum_kv::{KvError, Pool, ephemeral};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// WebAuthn challenge 有效期。
pub const CHALLENGE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum ChallengeError {
    #[error(transparent)]
    Kv(#[from] KvError),
    #[error("challenge state: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// `ns` 是部署的键前缀（`ForumCache::ns`）。
fn key(ns: &str, kind: &str, user_id: i64) -> String {
    format!("{ns}{kind}:{user_id}")
}

/// 保存一次仪式的状态；同一用途再次保存会覆盖（旧 challenge 随之作废）。
pub async fn save<T: Serialize>(
    pool: &Pool,
    ns: &str,
    kind: &str,
    user_id: i64,
    value: &T,
) -> Result<(), ChallengeError> {
    let json = serde_json::to_string(value)?;
    ephemeral::put(pool, &key(ns, kind, user_id), &json, CHALLENGE_TTL).await?;
    Ok(())
}

/// 取出并作废；过期或已用过返回 `None`。
pub async fn take<T: DeserializeOwned>(
    pool: &Pool,
    ns: &str,
    kind: &str,
    user_id: i64,
) -> Result<Option<T>, ChallengeError> {
    match ephemeral::take(pool, &key(ns, kind, user_id)).await? {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn pool() -> Option<Pool> {
        let url = forum_testkit::valkey_url()?;
        Some(forum_kv::pool(&url).unwrap())
    }

    fn user() -> i64 {
        rand::random::<u32>().into()
    }

    #[tokio::test]
    async fn taken_once() {
        let Some(p) = pool() else { return };
        let u = user();
        save(&p, "test:", "wallet:reg", u, &vec![1u8, 2, 3])
            .await
            .unwrap();
        let got: Option<Vec<u8>> = take(&p, "test:", "wallet:reg", u).await.unwrap();
        assert_eq!(got, Some(vec![1, 2, 3]));
        let again: Option<Vec<u8>> = take(&p, "test:", "wallet:reg", u).await.unwrap();
        assert_eq!(again, None, "cannot be replayed");
    }

    #[tokio::test]
    async fn kinds_and_namespaces_do_not_collide() {
        let Some(p) = pool() else { return };
        let u = user();
        save(&p, "a:", "wallet:auth", u, &"wallet").await.unwrap();
        save(&p, "a:", "arbitration:auth", u, &"ruling")
            .await
            .unwrap();
        save(&p, "b:", "wallet:auth", u, &"other deployment")
            .await
            .unwrap();
        // 更新的 challenge 覆盖同一用途的旧 challenge。
        save(&p, "a:", "wallet:auth", u, &"wallet, again")
            .await
            .unwrap();
        let take_str = |ns: &'static str, kind: &'static str| {
            let p = p.clone();
            async move { take::<String>(&p, ns, kind, u).await.unwrap() }
        };
        assert_eq!(
            take_str("a:", "wallet:auth").await.as_deref(),
            Some("wallet, again")
        );
        assert_eq!(
            take_str("a:", "arbitration:auth").await.as_deref(),
            Some("ruling")
        );
        assert_eq!(
            take_str("b:", "wallet:auth").await.as_deref(),
            Some("other deployment")
        );
    }

    #[tokio::test]
    async fn garbage_state_is_an_error() {
        let Some(p) = pool() else { return };
        let u = user();
        save(&p, "test:", "admin:reg", u, &"not a number")
            .await
            .unwrap();
        assert!(matches!(
            take::<u64>(&p, "test:", "admin:reg", u).await,
            Err(ChallengeError::Encoding(_))
        ));
    }
}
