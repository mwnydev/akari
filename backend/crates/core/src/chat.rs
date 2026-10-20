//! 私信、订单聊天与站内通知的共用结构（plan 第 4、7 节）：REST 响应与 WebSocket 推送用同一份 JSON。
//!
//! - 订单聊天哈希链：`hash = sha256(prev_hash ‖ canonical)`，第一条的 `prev_hash` 为 32 个 0 字节，
//!   `canonical` 见 [`canonical`]。聊天记录只追加（数据库触发器拒绝改删），裁决前用 [`verify`] 重算整条链。
//! - 风险提示：消息里出现钱包地址或「线下 / 私下转账」之类说法时标记 `warn`，界面提醒只走担保（[`needs_warning`]）。

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::floor::FloorComment;

/// 一条消息最多字数（私信与订单聊天相同）。
pub const MAX_MESSAGE_CHARS: usize = 2000;

/// 哈希链第一条消息的 `prev_hash`。
pub const GENESIS: [u8; 32] = [0; 32];

fn hex_bytes<S: serde::Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&alloy_primitives::hex::encode_prefixed(b))
}

/// 私信里的一条消息。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Message {
    pub id: i64,
    pub conversation_id: i64,
    /// 发送者；买家付款等由系统写入的卡片为空。
    pub sender_id: Option<i64>,
    /// `text` / `image` / `offer`（担保约定卡片）。
    pub kind: String,
    /// 文字内容；`offer` 卡片为事件：`created` / `modified` / `rejected` / `cancelled` / `paid` / `payment_reverted`。
    pub body: String,
    /// 图片键（图片上传上线后才有）。
    pub image: Option<String>,
    /// `offer` 卡片对应的报价（`GET /api/v1/offers/{id}` 取当前状态）。
    pub offer_id: Option<i64>,
    /// 含钱包地址或线下交易说法：界面提示只走担保。
    pub warn: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// 订单聊天里的一条消息。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OrderMessage {
    pub id: i64,
    pub order_id: i64,
    /// 系统消息（付款、发货、争议……）为空。
    pub sender_id: Option<i64>,
    /// `text` / `system` / `image`。
    pub kind: String,
    pub body: String,
    pub image: Option<String>,
    pub warn: bool,
    /// `0x` + 64 位十六进制。
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[serde(serialize_with = "hex_bytes")]
    pub prev_hash: Vec<u8>,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    #[serde(serialize_with = "hex_bytes")]
    pub hash: Vec<u8>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// 一条站内通知。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Notification {
    pub id: i64,
    /// `reply` / `mention` / `comment` / `like` / `message` / `order` / `offer` / `auction` / `system`。
    pub kind: String,
    /// 触发者（回复、@、评论的作者，最近点赞的人，报价的对方）；系统通知为空。
    pub actor_id: Option<i64>,
    /// 按类型：reply / mention 为 `{topic_id, post_id, floor}`（评论里的 @ 另有 `comment_id`）；
    /// comment 为 `{topic_id, post_id, floor, comment_id, reply}`（`reply` 为真：回复了你的评论，否则是评论了你的楼层）；
    /// like 为 `{topic_id, post_id, floor, count, likers}`（合并窗口内新增的赞数与最近几位，新的在前）；
    /// message 为 `{conversation_id, message_id}`；
    /// offer 为 `{offer_id, conversation_id, event}`；order 为 `{order_id, state, level, correction}`；
    /// auction 为 `{auction_id, topic_id, event, …}`（`event`：`outbid` 被超价、`won` 中标、`runner_up` 递补中标、
    /// `sold` / `unsold` 卖家的成交（待付款）与流拍、`defaulted` 逾期违约、`ending_soon` 出过价的拍卖即将结束（`ends_at`）、
    /// `pay_reminder` 付款截止时间快到了（`round`、`pay_deadline`），时间为 Unix 秒，见 `forum_db::auctions`）；
    /// system 为发件箱原样内容（含 `event`）。
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub payload: Value,
    pub read: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// WebSocket 服务端推送（`type` 区分）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// 私信新消息（双方所有在线设备）。
    Message {
        message: Message,
    },
    /// 私信已读回执。
    Read {
        conversation_id: i64,
        user_id: i64,
        last_id: i64,
    },
    /// 订单聊天新消息（双方与订阅了该订单的仲裁员）。
    OrderMessage {
        message: OrderMessage,
    },
    /// 订单聊天已读。
    OrderRead {
        order_id: i64,
        user_id: i64,
        last_id: i64,
    },
    /// 订单状态或确认级别变化；详情用 `GET /api/v1/orders/{id}` 取。
    Order {
        order_id: i64,
        state: String,
        level: String,
        /// 区块回滚后的状态修正。
        correction: bool,
    },
    Notification {
        notification: Notification,
    },
    /// 订阅的会话 / 订单里对方上下线。
    Presence(Presence),
    /// 订阅了主题页的连接：楼层下新增（或被恢复）一条评论；`comments` 为该楼现在的评论数。
    Comment {
        topic_id: i64,
        comment: FloorComment,
        comments: i32,
    },
    /// 订阅了主题页的连接：评论被删除。
    CommentRemoved {
        topic_id: i64,
        post_id: i64,
        comment_id: i64,
        comments: i32,
    },
    /// 订阅了主题页的连接：`user_id` 赞了（`liked`）或取消赞，`likes` 为该楼现在的点赞数。
    Likes {
        topic_id: i64,
        post_id: i64,
        user_id: i64,
        liked: bool,
        likes: i32,
    },
    /// 对方正在输入（只发给订阅了该会话 / 订单的连接）。
    Typing {
        conversation_id: Option<i64>,
        order_id: Option<i64>,
        user_id: i64,
    },
    /// 订阅了拍卖帖主题页的连接：新出价。金额为拍卖币种最小单位（十进制字符串），出价人已打码。
    AuctionBid {
        topic_id: i64,
        auction_id: i64,
        bid_id: i64,
        amount: String,
        bidder: String,
        /// 出价人与本场其他出价人是关联账号（同设备），提示其他用户。
        linked: bool,
        bids: i32,
        /// 顺延后的结束时间。
        #[serde(with = "time::serde::rfc3339")]
        ends_at: OffsetDateTime,
        reserve_met: bool,
    },
    /// 订阅了拍卖帖主题页的连接：拍卖状态变化（结束、成交、流拍……）；详情用 `GET /api/v1/auctions/{id}` 取。
    AuctionState {
        topic_id: i64,
        auction_id: i64,
        state: String,
    },
}

/// 在线状态。隐身用户对非交易对手只显示活跃到天（`last_seen_date`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Presence {
    pub user_id: i64,
    pub online: bool,
    /// 最近活跃时间（精确到分钟内）；隐身时为空。
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_seen_at: Option<OffsetDateTime>,
    /// 隐身时的最近活跃日期（UTC，`YYYY-MM-DD`）。
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = Date))]
    pub last_seen_date: Option<String>,
}

impl Presence {
    /// 按可见性生成：`real` 为真（本人、公开在线状态或进行中订单的交易对手）时给出真实状态。
    pub fn of(
        user_id: i64,
        online: bool,
        last_seen_at: Option<OffsetDateTime>,
        real: bool,
    ) -> Self {
        if real {
            return Self {
                user_id,
                online,
                last_seen_at,
                last_seen_date: None,
            };
        }
        Self {
            user_id,
            online: false,
            last_seen_at: None,
            last_seen_date: last_seen_at.map(|t| t.date().to_string()),
        }
    }
}

/// 参与哈希的消息内容。字段顺序固定；时间为 Unix 微秒（与 PostgreSQL 精度一致）。
#[derive(Serialize)]
struct Canonical<'a> {
    v: u8,
    order_id: i64,
    sender_id: Option<i64>,
    kind: &'a str,
    body: &'a str,
    image: Option<&'a str>,
    created_at: i128,
}

/// 消息的规范字节：`{"v":1,"order_id":…,"sender_id":…,"kind":…,"body":…,"image":…,"created_at":微秒}`
/// （serde_json 紧凑输出，字符串按 JSON 转义）。
pub fn canonical(
    order_id: i64,
    sender_id: Option<i64>,
    kind: &str,
    body: &str,
    image: Option<&str>,
    created_at: OffsetDateTime,
) -> Vec<u8> {
    let c = Canonical {
        v: 1,
        order_id,
        sender_id,
        kind,
        body,
        image,
        created_at: created_at.unix_timestamp_nanos() / 1000,
    };
    // 只含整数与字符串的结构体序列化不会失败。
    serde_json::to_vec(&c).unwrap_or_default()
}

/// `sha256(prev_hash ‖ canonical)`。
pub fn chain_hash(prev_hash: &[u8], canonical: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev_hash);
    h.update(canonical);
    h.finalize().into()
}

/// 截到微秒：写入 PostgreSQL 再读出后哈希不变。
pub fn now_micros() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(now.nanosecond() / 1000 * 1000)
        .unwrap_or(now)
}

impl OrderMessage {
    /// 按存储的内容重算本条哈希。
    pub fn recompute(&self) -> [u8; 32] {
        chain_hash(
            &self.prev_hash,
            &canonical(
                self.order_id,
                self.sender_id,
                &self.kind,
                &self.body,
                self.image.as_deref(),
                self.created_at,
            ),
        )
    }
}

/// 哈希链校验的进度：按 ID 顺序逐条喂入 [`ChainCheck::push`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainCheck {
    head: [u8; 32],
    pub count: u64,
    /// 第一条对不上的消息 ID（内容被改、被删或顺序被动过）。
    pub broken_at: Option<i64>,
}

impl Default for ChainCheck {
    fn default() -> Self {
        Self {
            head: GENESIS,
            count: 0,
            broken_at: None,
        }
    }
}

impl ChainCheck {
    pub fn push(&mut self, m: &OrderMessage) {
        self.count += 1;
        if self.broken_at.is_some() {
            return;
        }
        let hash = m.recompute();
        if m.prev_hash != self.head || hash != m.hash.as_slice() {
            self.broken_at = Some(m.id);
            return;
        }
        self.head = hash;
    }

    /// 链头（最后一条消息的哈希；空链为全 0）。
    pub fn head(&self) -> [u8; 32] {
        self.head
    }
}

/// 一次性校验整条链。
pub fn verify<'a>(messages: impl IntoIterator<Item = &'a OrderMessage>) -> ChainCheck {
    let mut c = ChainCheck::default();
    for m in messages {
        c.push(m);
    }
    c
}

/// 「绕开担保」的常见说法。
const OFF_PLATFORM: &[&str] = &[
    "线下",
    "私下",
    "场外",
    "直接转账",
    "直接打款",
    "不走担保",
    "绕过担保",
];

/// 是否需要「只走担保」提示：含 EVM 地址（`0x` + 40 位十六进制）、TRON 地址（`T` + 33 位 Base58）或线下交易说法。
pub fn needs_warning(body: &str) -> bool {
    OFF_PLATFORM.iter().any(|k| body.contains(k)) || has_evm_address(body) || has_tron_address(body)
}

fn has_evm_address(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(41)).any(|i| {
        b[i] == b'0'
            && (b[i + 1] | 0x20) == b'x'
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
            && b[i + 2..i + 42].iter().all(u8::is_ascii_hexdigit)
            && b.get(i + 42).is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

fn is_base58(c: u8) -> bool {
    c.is_ascii_alphanumeric() && !matches!(c, b'0' | b'O' | b'I' | b'l')
}

fn has_tron_address(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(33)).any(|i| {
        b[i] == b'T'
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
            && b[i + 1..i + 34].iter().all(|&c| is_base58(c))
            && b.get(i + 34).is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn msg(id: i64, prev: [u8; 32], body: &str) -> OrderMessage {
        let created_at = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let mut m = OrderMessage {
            id,
            order_id: 7,
            sender_id: Some(3),
            kind: "text".into(),
            body: body.into(),
            image: None,
            warn: false,
            prev_hash: prev.to_vec(),
            hash: Vec::new(),
            created_at,
        };
        m.hash = m.recompute().to_vec();
        m
    }

    fn chain(bodies: &[&str]) -> Vec<OrderMessage> {
        let mut prev = GENESIS;
        let mut out = Vec::new();
        for (i, b) in bodies.iter().enumerate() {
            let m = msg(i64::try_from(i).unwrap() + 1, prev, b);
            prev = m.hash.clone().try_into().unwrap();
            out.push(m);
        }
        out
    }

    #[test]
    fn canonical_bytes_are_stable() {
        let t = OffsetDateTime::from_unix_timestamp_nanos(1_800_000_000_123_456_000).unwrap();
        assert_eq!(
            String::from_utf8(canonical(7, None, "system", "已\"付款\"", None, t)).unwrap(),
            r#"{"v":1,"order_id":7,"sender_id":null,"kind":"system","body":"已\"付款\"","image":null,"created_at":1800000000123456}"#
        );
        // sha256(32 个 0 字节 ‖ "a")，与 `(head -c 32 /dev/zero; printf a) | sha256sum` 一致
        assert_eq!(
            alloy_primitives::hex::encode(chain_hash(&GENESIS, b"a")),
            "41a0370c3d9f42773a59e8e01651911cf43b1e3f66944cbb690029debc4eb647"
        );
    }

    #[test]
    fn verify_detects_tampering() {
        let good = chain(&["a", "b", "c"]);
        let ok = verify(&good);
        assert_eq!((ok.count, ok.broken_at), (3, None));
        assert_eq!(ok.head().as_slice(), good[2].hash.as_slice());

        let mut edited = good.clone();
        edited[1].body = "B".into();
        assert_eq!(verify(&edited).broken_at, Some(2));

        let mut removed = good.clone();
        removed.remove(1);
        assert_eq!(verify(&removed).broken_at, Some(3));

        // 改内容同时重算本条哈希也没用：下一条的 prev_hash 对不上
        let mut rehashed = good;
        rehashed[0].body = "x".into();
        rehashed[0].hash = rehashed[0].recompute().to_vec();
        assert_eq!(verify(&rehashed).broken_at, Some(2));

        assert_eq!(verify(&[]).head(), GENESIS);
    }

    #[test]
    fn warnings() {
        let addr = format!("0x{}", "aB".repeat(20));
        assert!(needs_warning(&format!("打到 {addr} 就行")));
        assert!(needs_warning(&addr));
        assert!(
            !needs_warning(&format!("{addr}00")),
            "tx hash, not an address"
        );
        assert!(!needs_warning(&format!("0x{}", "a".repeat(39))));
        assert!(needs_warning("TQn9Y2khEsLJW1ChVWFMSMeRDow5KcbLSE"));
        assert!(!needs_warning("TQn9Y2khEsLJW1ChVWFMSMeRDow5KcbLS"));
        assert!(needs_warning("我们私下转账吧"));
        assert!(needs_warning("线下交易更便宜"));
        assert!(!needs_warning("明天发货，请注意查收"));
        assert!(!needs_warning(""));
    }

    #[test]
    fn hidden_presence_only_shows_the_day() {
        let t = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let real = Presence::of(1, true, Some(t), true);
        assert!(real.online && real.last_seen_at == Some(t) && real.last_seen_date.is_none());
        let hidden = Presence::of(1, true, Some(t), false);
        assert!(!hidden.online && hidden.last_seen_at.is_none());
        assert_eq!(hidden.last_seen_date.as_deref(), Some("2027-01-15"));
    }
}
