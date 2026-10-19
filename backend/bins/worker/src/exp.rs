//! 发帖经验（plan 第 7 节）：帖子发出满 24 小时、未被删除才计；按帖子 ID 顺序扫描，进度存 `job_cursors`。

use forum_db::exp::{self, DuePost, Grant};
use forum_db::{PgPool, jobs, posts, topics};
use forum_kv::ForumCache;
use sha2::{Digest, Sha256};

/// 扫描间隔。
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
const CURSOR: &str = "exp.posts";
const BATCH: i64 = 500;

/// 这条帖子按什么规则计经验、同一来源的去重键；不计返回 `None`。
fn rule_for(p: &DuePost) -> Option<(&'static exp::Rule, String)> {
    if p.status != posts::STATUS_NORMAL || p.topic_flags & topics::FLAG_HIDDEN != 0 {
        return None;
    }
    if p.floor == 1 {
        return Some((&exp::TOPIC, format!("post:{}", p.id)));
    }
    // 回复：回自己的主题、没有文字（纯表情、纯图片）不计；同样的内容只计一次
    if p.author_id == p.topic_author_id || !p.plain_text.chars().any(char::is_alphanumeric) {
        return None;
    }
    let normalized: String = p
        .plain_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    Some((&exp::REPLY, format!("text:{}", hex::encode(&digest[..16]))))
}

/// 处理所有已满 24 小时的帖子，返回本次扫描的帖子数。另一个 worker 正在处理时直接返回 0。
pub async fn award_posts(db: &PgPool, cache: &ForumCache) -> Result<usize, sqlx::Error> {
    let mut total = 0;
    loop {
        let mut tx = db.begin().await?;
        let Some(after) = jobs::lock_cursor(&mut tx, CURSOR).await? else {
            return Ok(total);
        };
        let due = exp::posts_due(&mut tx, after, BATCH).await?;
        let Some(last) = due.last().map(|p| p.id) else {
            return Ok(total);
        };
        let mut level_up = Vec::new();
        for p in &due {
            let Some((rule, key)) = rule_for(p) else {
                continue;
            };
            let g = Grant {
                user_id: p.author_id,
                ref_id: Some(p.id),
                day: p.created_at.date(),
                dedupe: Some(&key),
            };
            if exp::award(&mut tx, rule, g).await? == Some(true) {
                level_up.push(p.author_id);
            }
        }
        jobs::set_cursor(&mut tx, CURSOR, last).await?;
        tx.commit().await?;
        if let Err(e) = cache.cards_invalidate(&level_up).await {
            tracing::warn!(error = %e, "user card invalidation failed");
        }
        total += due.len();
        if due.len() < BATCH as usize {
            return Ok(total);
        }
    }
}
