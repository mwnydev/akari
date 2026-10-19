//! 论坛接口的响应结构。时间一律 RFC 3339（UTC）。

use forum_db::{forums::Forum, posts::Post, topics, users::Card};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ForumView {
    pub id: i32,
    pub parent_id: Option<i32>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub position: i32,
    /// 可见 / 发帖 / 回复所需最低等级（游客为 0）。
    pub view_min_level: i16,
    pub post_min_level: i16,
    pub reply_min_level: i16,
}

impl From<&Forum> for ForumView {
    fn from(f: &Forum) -> Self {
        Self {
            id: f.id,
            parent_id: f.parent_id,
            slug: f.slug.clone(),
            name: f.name.clone(),
            description: f.description.clone(),
            position: f.position,
            view_min_level: f.view_min_level,
            post_min_level: f.post_min_level,
            reply_min_level: f.reply_min_level,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TopicFlags {
    pub pinned: bool,
    pub featured: bool,
    pub locked: bool,
    pub hidden: bool,
}

impl From<i16> for TopicFlags {
    fn from(f: i16) -> Self {
        Self {
            pinned: f & topics::FLAG_PINNED != 0,
            featured: f & topics::FLAG_FEATURED != 0,
            locked: f & topics::FLAG_LOCKED != 0,
            hidden: f & topics::FLAG_HIDDEN != 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopicSummary {
    pub id: i64,
    pub forum_id: i32,
    pub author_id: i64,
    pub title: String,
    /// 0=普通、1=交易、2=争议。
    pub kind: i16,
    pub flags: TopicFlags,
    /// 回复数（楼层数 − 1）。
    pub replies: i32,
    pub last_floor: i32,
    pub last_poster_id: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    pub last_post_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub views: i64,
}

impl From<&topics::Topic> for TopicSummary {
    fn from(t: &topics::Topic) -> Self {
        Self {
            id: t.id,
            forum_id: t.forum_id,
            author_id: t.author_id,
            title: t.title.clone(),
            kind: t.kind,
            flags: t.flags.into(),
            replies: t.last_floor - 1,
            last_floor: t.last_floor,
            last_poster_id: t.last_poster_id,
            last_post_at: t.last_post_at,
            created_at: t.created_at,
            views: t.view_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserCardView {
    pub id: i64,
    /// 还没设置用户名时为空，前端显示「用户 {id}」。
    pub username: Option<String>,
    pub level: i16,
}

impl From<Card> for UserCardView {
    fn from(c: Card) -> Self {
        Self {
            id: c.id,
            username: c.username,
            level: c.level,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PostView {
    pub id: i64,
    pub floor: i32,
    pub author_id: i64,
    /// 已过滤的 HTML；已删除的楼层为空字符串。
    pub html: String,
    pub deleted: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub edited_at: Option<OffsetDateTime>,
}

impl From<Post> for PostView {
    fn from(p: Post) -> Self {
        let deleted = p.status != forum_db::posts::STATUS_NORMAL;
        Self {
            id: p.id,
            floor: p.floor,
            author_id: p.author_id,
            html: if deleted { String::new() } else { p.html },
            deleted,
            created_at: p.created_at,
            edited_at: p.edited_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BoardResponse {
    pub forum: ForumView,
    /// 置顶帖，只在第 1 页返回。
    pub pinned: Vec<TopicSummary>,
    pub topics: Vec<TopicSummary>,
    /// 列表里出现的作者与最后回复者。
    pub users: Vec<UserCardView>,
    /// 下一页页码（前 200 页）；之后改用 `next_cursor`。
    pub next_page: Option<u32>,
    pub next_cursor: Option<String>,
}

/// 主题页里与访问者无关、可以缓存的部分。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopicPageBody {
    pub forum: ForumView,
    pub topic: TopicSummary,
    pub posts: Vec<PostView>,
    pub users: Vec<UserCardView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopicPage {
    #[serde(flatten)]
    pub body: TopicPageBody,
    pub page: i32,
    pub total_pages: i32,
    pub can_reply: bool,
    pub can_moderate: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedTopic {
    pub topic_id: i64,
    pub post_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedPost {
    pub post_id: i64,
    pub floor: i32,
    /// 新楼层所在页码。
    pub page: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ModerationResult {
    pub forum_id: i32,
    pub flags: TopicFlags,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EditView {
    pub id: i64,
    pub editor_id: i64,
    /// 1 楼编辑时的旧标题。
    pub title: Option<String>,
    /// 编辑前的 HTML。
    pub html: String,
    #[serde(with = "time::serde::rfc3339")]
    pub edited_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EditHistory {
    pub edits: Vec<EditView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PostSource {
    #[schema(value_type = Object)]
    pub source: serde_json::Value,
}

/// 字节串的 `0x` 小写十六进制形式（地址、哈希、签名、公钥坐标）。
pub fn hex0x(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}
