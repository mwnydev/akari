//! 论坛接口的响应结构。时间一律 RFC 3339（UTC）。

use forum_db::users::{Card, TradeCounts};
use forum_db::{forums::Forum, posts::Post, topics};
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
    /// 能否发交易帖。
    pub allow_trade: bool,
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
            allow_trade: f.allow_trade,
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
    /// 交易帖的交易帖 ID 与价格（USDT 最小单位，十进制字符串）；普通主题为空。
    pub listing_id: Option<i64>,
    pub price: Option<String>,
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
            listing_id: t.listing_id,
            price: t.price.clone(),
        }
    }
}

/// 成功率至少要有多少笔计入的订单才显示（plan 第 7 节：不满 10 笔显示「新卖家 · 已成交 N 笔」）。
pub const MIN_RATED_TRADES: i32 = 10;

/// 名片上的交易信誉（plan 第 7 节；只统计最终确认的链上订单）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ReputationView {
    /// 成交笔数（成功订单，卖家或买家身份）。
    pub success: i32,
    /// 本人责任的失败笔数（作为卖家 + 作为买家）。
    pub failed: i32,
    pub disputes_lost: i32,
    /// 协商和解等不计入成功率的次数。
    pub settled: i32,
    /// 成功率（万分比）= 成功 ÷（成功 + 本人责任失败）；不满 10 笔为空。
    pub success_rate_bps: Option<u32>,
}

impl From<TradeCounts> for ReputationView {
    fn from(t: TradeCounts) -> Self {
        let failed = t.seller_fault_fail + t.buyer_fault_fail;
        let rated = t.success + failed;
        Self {
            success: t.success,
            failed,
            disputes_lost: t.disputes_lost,
            settled: t.settled,
            success_rate_bps: (rated >= MIN_RATED_TRADES)
                .then(|| u32::try_from(i64::from(t.success) * 10_000 / i64::from(rated)).ok())
                .flatten(),
        }
    }
}

/// 用户名片：名字、头像、等级、信誉（plan 第 14 节「用户名片」组件的数据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserCardView {
    pub id: i64,
    /// 还没设置用户名时为空，前端显示「用户 {id}」。
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub level: i16,
    pub reputation: ReputationView,
}

impl From<Card> for UserCardView {
    fn from(c: Card) -> Self {
        Self {
            id: c.id,
            username: c.username,
            avatar_url: c.avatar_url,
            level: c.level,
            reputation: c.trade.into(),
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

#[derive(Debug, Serialize, ToSchema)]
pub struct LatestResponse {
    /// 全站最新回复的普通主题（不含置顶与隐藏），只来自访问者看得到的板块。
    pub topics: Vec<TopicSummary>,
    /// 这些主题所在的板块。
    pub forums: Vec<ForumView>,
    pub users: Vec<UserCardView>,
    pub next_cursor: Option<String>,
}

/// 主题页里与访问者无关、可以缓存的部分。名片不在里面：名片有自己的缓存，改名、升级后立即可见。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopicPageBody {
    pub forum: ForumView,
    pub topic: TopicSummary,
    pub posts: Vec<PostView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopicPage {
    #[serde(flatten)]
    pub body: TopicPageBody,
    /// 主题作者与本页各楼层作者。
    pub users: Vec<UserCardView>,
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
