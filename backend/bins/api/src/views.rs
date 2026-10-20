//! 论坛接口的响应结构。时间一律 RFC 3339（UTC）。

use forum_db::floor::FloorComment;
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
    /// 板块色块：固定色板里的 `#rrggbb`；为空时前端按排序位置轮换。
    pub color: Option<String>,
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
            color: f.color.clone(),
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
    /// 0=普通、1=交易、2=争议、3=拍卖。
    pub kind: i16,
    pub flags: TopicFlags,
    /// 回复数（楼层数 − 1）。
    pub replies: i32,
    /// 楼层数（时间轴的总层数；首尾时间为 `created_at` 与 `last_post_at`）。
    pub last_floor: i32,
    pub last_poster_id: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    pub last_post_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub views: i64,
    /// 列表行的参与者头像：楼主在前，之后是最近发言的人（新的在前，不重复），最多 5 个；名片在响应的 `users` 里。
    pub participants: Vec<i64>,
    /// 交易帖的信息行；普通主题为空。
    pub trade: Option<TradeSummary>,
    /// 拍卖帖的信息行（当前价、出价次数、结束时间等）；只在主题列表里给出，主题页用 `GET /api/v1/topics/{id}/auction`。
    pub auction: Option<AuctionSummary>,
}

/// 主题列表里交易帖的信息行。卖家就是楼主，成功率等信誉见 `users` 里楼主的名片（`reputation`）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TradeSummary {
    pub listing_id: i64,
    /// 价格，USDT 最小单位（十进制字符串）。
    pub price: String,
    /// 卖家参考汇率（ADR 0011，每 1 USDT 合多少元的 4 位小数定点数）；不填只显示 USDT。
    pub cny_per_usdt: Option<i32>,
    /// 自动发货（付款后即时交付）；否则卖家人工发货。
    pub instant: bool,
    /// 剩余库存：仍有效的公开报价还能成交的次数。没有公开报价时为空；只在主题列表里给出。
    pub stock: Option<i64>,
}

/// 拍卖帖的信息行（ADR 0011）。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuctionSummary {
    pub auction_id: i64,
    /// `live` / `ended` / `awaiting_payment` / `settled` / `unsold` / `cancelled`（同 `AuctionView.state`）。
    pub state: String,
    /// `usdt` 或 `cny`。
    pub currency: String,
    /// 当前价：USDT 时为最小单位，人民币时为分（十进制字符串）。
    pub current: String,
    /// 人民币计价时卖家签名的汇率。
    pub cny_per_usdt: Option<i32>,
    pub bids: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub ends_at: OffsetDateTime,
    pub reserve_met: bool,
}

/// 列表行最多显示几个参与者头像（含楼主）。
const MAX_PARTICIPANTS: usize = 5;

impl From<&topics::Topic> for TopicSummary {
    fn from(t: &topics::Topic) -> Self {
        let participants = std::iter::once(t.author_id)
            .chain(
                t.recent_posters
                    .iter()
                    .copied()
                    .filter(|&u| u != t.author_id),
            )
            .take(MAX_PARTICIPANTS)
            .collect();
        let trade = match (t.listing_id, &t.price) {
            (Some(listing_id), Some(price)) => Some(TradeSummary {
                listing_id,
                price: price.clone(),
                cny_per_usdt: t.cny_per_usdt,
                instant: t.instant.unwrap_or(false),
                stock: None,
            }),
            _ => None,
        };
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
            participants,
            trade,
            auction: None,
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
    /// 已过滤的 HTML；已删除的楼层，以及访问者看不到的隐藏楼层为空字符串。
    pub html: String,
    pub deleted: bool,
    /// 被管理人员隐藏：管理人员与作者看得到内容（界面上带标记 / 提示），其他人只看到「已隐藏」。
    /// （`serde(default)` 只为读取本字段加入之前缓存的主题页，接口总会返回。）
    #[serde(default)]
    #[schema(required = true)]
    pub hidden: bool,
    /// 隐藏理由，只给管理人员与作者本人。
    #[serde(default)]
    #[schema(required = true)]
    pub hidden_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub edited_at: Option<OffsetDateTime>,
}

impl PostView {
    /// 楼层与它的隐藏理由（隐藏楼层才有）。隐藏楼层保留内容，按访问者裁剪见 [`Self::redact`]。
    pub fn new(p: Post, hidden_reason: Option<String>) -> Self {
        let hidden = p.status == forum_db::posts::STATUS_HIDDEN;
        let deleted = p.status != forum_db::posts::STATUS_NORMAL && !hidden;
        Self {
            id: p.id,
            floor: p.floor,
            author_id: p.author_id,
            html: if deleted { String::new() } else { p.html },
            deleted,
            hidden,
            hidden_reason: hidden_reason.filter(|_| hidden),
            created_at: p.created_at,
            edited_at: p.edited_at,
        }
    }

    /// 访问者能看到这一楼的内容：未删除，且不是隐藏楼层或访问者是管理人员 / 作者。
    pub fn readable(&self, staff: bool, viewer: Option<i64>) -> bool {
        !self.deleted && (!self.hidden || staff || viewer == Some(self.author_id))
    }

    /// 按访问者裁剪隐藏楼层：看不到的去掉内容与理由。
    pub fn redact(&mut self, staff: bool, viewer: Option<i64>) {
        if self.hidden && !self.readable(staff, viewer) {
            self.html.clear();
            self.hidden_reason = None;
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

/// 一楼下方的社交区：点赞与最早的几条评论（朋友圈式，按访问者计算，不进主题页缓存）。
#[derive(Debug, Serialize, ToSchema)]
pub struct FloorSocialView {
    pub post_id: i64,
    pub likes: i32,
    /// 访问者是否已赞（游客为假）。
    pub liked: bool,
    /// 最近点赞的几个人（最多 5 个，新的在前），其余显示「等 N 人」。
    pub likers: Vec<i64>,
    /// 未删除的评论总数。
    pub comment_count: i32,
    /// 最早的 5 条评论，按时间正序；其余用 `GET /api/v1/posts/{id}/comments?after=` 取。
    pub comments: Vec<FloorComment>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopicPage {
    #[serde(flatten)]
    pub body: TopicPageBody,
    /// 本页各楼的点赞与评论，顺序同 `posts`（已删除的楼层为空）。
    pub social: Vec<FloorSocialView>,
    /// 主题作者、本页各楼层作者，以及点赞名单与评论里出现的人。
    pub users: Vec<UserCardView>,
    pub page: i32,
    pub total_pages: i32,
    pub can_reply: bool,
    pub can_moderate: bool,
}

/// 一楼的评论翻页。
#[derive(Debug, Serialize, ToSchema)]
pub struct CommentPage {
    pub comments: Vec<FloorComment>,
    /// 评论作者与被回复的人。
    pub users: Vec<UserCardView>,
    /// 还有更多时为下一页的 `after`。
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreatedComment {
    pub comment: FloorComment,
    /// 该楼现在的评论数。
    pub comment_count: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LikeState {
    pub liked: bool,
    /// 该楼现在的点赞数。
    pub likes: i32,
}

/// 楼层时间轴（主题页右侧的滑块）：首尾时间与均匀采样的楼层时间，中间的楼层按相邻采样点插值。
#[derive(Debug, Serialize, ToSchema)]
pub struct Timeline {
    pub last_floor: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub first_post_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_post_at: OffsetDateTime,
    /// 楼层号升序，含 1 楼与最后一楼；楼层不多时每层都有。
    pub points: Vec<TimelinePoint>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimelinePoint {
    pub floor: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
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
