//! 全文搜索（plan 第 8 节）：楼层正文与主题标题走 PGroonga（[`forum_db::search`]），用户按用户名前缀。
//!
//! - 只返回访问者看得到的内容：楼层未删除、主题未隐藏、板块在访问者可见范围内（[`visible_forums`]）。
//! - 按 id 倒序（最新在前），游标翻页；一个查询最多翻到 [`MAX_RESULTS`] 条。
//! - 高亮由服务器切成「文字 / 命中」片段，前端当纯文本渲染，不拼 HTML。
//! - 单独限流（游客按 IP、登录用户按账号，游客更紧），每个进程同时最多 [`MAX_CONCURRENT`] 个搜索，单条语句 3 秒超时。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use forum_core::ErrorCode;
use forum_db::search::{self, Hits, SearchError};
use forum_db::topics::{self, FLAG_HIDDEN};
use forum_kv::SlidingWindow;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};

use crate::access::{Viewer, visible_forums};
use crate::cards;
use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::ClientIp;
use crate::routes::forums::{summaries, topic_user_ids};
use crate::routes::users::{MentionView, search_prefix};
use crate::state::AppState;
use crate::views::{ForumView, TopicSummary, UserCardView};

/// 每页条数。
const PER_PAGE: usize = 20;
/// 一个查询最多翻到多少条（plan 第 8 节「结果最多 1000 条」）。
pub const MAX_RESULTS: usize = 1000;
/// 每轮从 Groonga 取多少个候选（回表过滤掉删除、隐藏与看不到的板块后不够一页就再取，后一轮更多）。
const POST_ROUNDS: [i64; 3] = [
    3 * PER_PAGE as i64,
    10 * PER_PAGE as i64,
    30 * PER_PAGE as i64,
];
/// 主题的可见板块已在 Groonga 里过滤，只剩隐藏主题要回表过滤。
const TOPIC_ROUNDS: [i64; 2] = [2 * PER_PAGE as i64, 10 * PER_PAGE as i64];
/// 每个进程同时进行的搜索数（每个占一个数据库连接，连接池共 20 个）；排不上队超过 [`QUEUE_WAIT`] 就返回 429。
const MAX_CONCURRENT: usize = 6;
const QUEUE_WAIT: Duration = Duration::from_secs(2);
/// 片段长度（字符）与命中前保留的上文。
const SNIPPET_CHARS: usize = 160;
const SNIPPET_LEAD: usize = 40;

/// 游客按 IP：每分钟 20 次；登录用户按账号：每分钟 60 次（初始值，按压测结果调整）。
pub const GUEST_SEARCH_LIMIT: u32 = 20;
pub const GUEST_SEARCH_RATE: SlidingWindow =
    SlidingWindow::new("search-guest", GUEST_SEARCH_LIMIT, Duration::from_secs(60));
pub const USER_SEARCH_RATE: SlidingWindow =
    SlidingWindow::new("search-user", 60, Duration::from_secs(60));

static SLOTS: Semaphore = Semaphore::const_new(MAX_CONCURRENT);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    /// 楼层正文（默认）。
    #[default]
    Post,
    /// 主题标题。
    Topic,
    /// 用户名前缀。
    User,
    /// 搜索页默认的「全部」：主题标题（前 [`ALL_TOPICS`] 条）、楼层正文（前 [`ALL_POSTS`] 条）与用户名前缀
    /// （关键词像用户名时）一起返回，各带命中数，不翻页（翻页切到单一类型）。只算一次限流。
    All,
}

/// 「全部」里每种最多几条。
const ALL_TOPICS: usize = 5;
const ALL_POSTS: usize = 10;
const ALL_PEOPLE: usize = 5;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    /// 关键词，空格分隔的多个词须同时出现；最多 64 个字符、5 个词。
    pub q: String,
    #[serde(rename = "type", default)]
    #[param(rename = "type", inline)]
    pub kind: SearchType,
    /// 只搜这个板块（slug）。
    pub board: Option<String>,
    /// 上一页的 `next_cursor`。
    pub cursor: Option<String>,
}

/// 高亮片段：`hit` 为真的是命中的词。前端按纯文本渲染。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Highlight {
    pub text: String,
    pub hit: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PostResult {
    pub id: i64,
    pub topic_id: i64,
    pub topic_title: String,
    pub forum_id: i32,
    pub floor: i32,
    /// 楼层所在的页（每页 20 楼）。
    pub page: i32,
    pub author_id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// 正文里第一处命中附近的约 160 字，换行变空格；前后被截断时 `truncated_*` 为真。
    pub snippet: Vec<Highlight>,
    pub truncated_start: bool,
    pub truncated_end: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopicResult {
    pub topic: TopicSummary,
    /// 标题的高亮。
    pub title: Vec<Highlight>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    #[serde(rename = "type")]
    pub kind: SearchType,
    /// 从本页起往前的命中数估计（含访问者看不到的，翻页时变小；用户搜索为结果数；`all` 为楼层命中数）。
    pub total: i64,
    /// 主题标题的命中数估计：`topic` 与 `all` 才有，其余为 0。
    pub topic_total: i64,
    pub posts: Vec<PostResult>,
    pub topics: Vec<TopicResult>,
    pub people: Vec<MentionView>,
    /// 结果里出现的板块。
    pub forums: Vec<ForumView>,
    /// 结果里出现的作者等的名片。
    pub users: Vec<UserCardView>,
    pub next_cursor: Option<String>,
}

/// 游标：`{扫描到的 id}.{已返回条数}`。
fn parse_cursor(c: &str) -> ApiResult<(i64, usize)> {
    let bad = || ApiError(ErrorCode::InvalidParameter);
    let (id, served) = c.split_once('.').ok_or_else(bad)?;
    Ok((
        id.parse().map_err(|_| bad())?,
        served.parse().map_err(|_| bad())?,
    ))
}

impl From<SearchError> for ApiError {
    fn from(e: SearchError) -> Self {
        Self::internal(e)
    }
}

/// 搜索楼层、主题或用户。
#[utoipa::path(
    get, path = "/api/v1/search", tag = "forum",
    params(SearchQuery),
    responses(
        (status = 200, body = SearchResponse),
        (status = 400, body = ErrorBody), (status = 404, body = ErrorBody), (status = 429, body = ErrorBody)
    )
)]
pub async fn search(
    State(state): State<AppState>,
    session: Session,
    ClientIp(ip): ClientIp,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<SearchResponse>> {
    let viewer = Viewer::load(&state, &session).await?;
    let allowed = match viewer.user {
        Some(u) => {
            let id = format!("{}{}", state.cache.ns(), u.id);
            USER_SEARCH_RATE.hit(&state.kv, &id).await?
        }
        None => {
            let id = format!("{}{ip}", state.cache.ns());
            GUEST_SEARCH_RATE.hit(&state.kv, &id).await?
        }
    };
    if !allowed {
        return Err(ApiError(ErrorCode::RateLimited));
    }
    let mut res = SearchResponse {
        kind: q.kind,
        total: 0,
        topic_total: 0,
        posts: Vec::new(),
        topics: Vec::new(),
        people: Vec::new(),
        forums: Vec::new(),
        users: Vec::new(),
        next_cursor: None,
    };
    if q.kind == SearchType::User {
        res.people = search_prefix(&state, &q.q).await?;
        res.total = res.people.len() as i64;
        return Ok(Json(res));
    }
    let Some((query, terms)) = search::groonga_query(&q.q) else {
        return Err(ApiError(ErrorCode::InvalidParameter));
    };
    let (before, served) = match q.cursor.as_deref() {
        Some(c) => {
            let (id, served) = parse_cursor(c)?;
            (Some(id), served)
        }
        None => (None, 0),
    };
    if q.kind == SearchType::All && q.cursor.is_some() {
        return Err(ApiError(ErrorCode::InvalidParameter));
    }
    if served >= MAX_RESULTS {
        return Ok(Json(res));
    }

    let mut forums = visible_forums(&state, &viewer).await?;
    if let Some(slug) = q.board.as_deref() {
        forums.retain(|f| f.slug == slug);
        if forums.is_empty() {
            return Err(ApiError(ErrorCode::NotFound));
        }
    }
    let forum_ids: Vec<i32> = forums.iter().map(|f| f.id).collect();

    let _slot = tokio::time::timeout(QUEUE_WAIT, SLOTS.acquire())
        .await
        .map_err(|_| ApiError(ErrorCode::RateLimited))?
        .map_err(ApiError::internal)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    let found = match q.kind {
        SearchType::Topic => vec![find_topics(&mut tx, &query, &forum_ids, before).await?],
        SearchType::All => vec![
            find_topics(&mut tx, &query, &forum_ids, None).await?,
            find_posts(&mut tx, &query, &forum_ids, None).await?,
        ],
        SearchType::Post | SearchType::User => {
            vec![find_posts(&mut tx, &query, &forum_ids, before).await?]
        }
    };
    tx.commit().await?;

    let mut user_ids = Vec::new();
    let mut forum_seen = HashSet::new();
    for found in found {
        if q.kind == SearchType::All {
            fill(
                &state,
                &mut res,
                found.cut(),
                &terms,
                &mut user_ids,
                &mut forum_seen,
            )
            .await?;
        } else {
            let shown = served + found.ids.len();
            res.next_cursor = found
                .next
                .filter(|_| shown < MAX_RESULTS)
                .map(|id| format!("{id}.{shown}"));
            fill(
                &state,
                &mut res,
                found,
                &terms,
                &mut user_ids,
                &mut forum_seen,
            )
            .await?;
        }
    }
    if q.kind == SearchType::Topic {
        res.total = res.topic_total;
    }
    if q.kind == SearchType::All {
        res.people = search_prefix(&state, &q.q).await?;
        res.people.truncate(ALL_PEOPLE);
    }
    res.forums = forums
        .iter()
        .filter(|f| forum_seen.contains(&f.id))
        .map(ForumView::from)
        .collect();
    res.users = cards::load(&state, &user_ids).await?;
    Ok(Json(res))
}

/// 把一种结果填进回应：命中数（楼层记在 `total`，主题记在 `topic_total`）、条目、要带的名片与板块。
async fn fill(
    state: &AppState,
    res: &mut SearchResponse,
    found: Found,
    terms: &[String],
    user_ids: &mut Vec<i64>,
    forum_seen: &mut HashSet<i32>,
) -> ApiResult<()> {
    match found.rows {
        Rows::Posts(rows) => {
            res.total = found.total;
            for p in rows {
                let (snippet, truncated_start, truncated_end) = snippet(&p.plain_text, terms);
                user_ids.push(p.author_id);
                forum_seen.insert(p.forum_id);
                res.posts.push(PostResult {
                    id: p.id,
                    topic_id: p.topic_id,
                    topic_title: p.title,
                    forum_id: p.forum_id,
                    floor: p.floor,
                    page: (p.floor - 1) / forum_db::posts::PAGE_SIZE + 1,
                    author_id: p.author_id,
                    created_at: p.created_at,
                    snippet,
                    truncated_start,
                    truncated_end,
                });
            }
        }
        Rows::Topics(rows) => {
            res.topic_total = found.total;
            user_ids.extend(topic_user_ids(rows.iter()));
            forum_seen.extend(rows.iter().map(|t| t.forum_id));
            for summary in summaries(state, &rows).await? {
                let title = highlight(&summary.title, terms);
                res.topics.push(TopicResult {
                    topic: summary,
                    title,
                });
            }
        }
    }
    Ok(())
}

enum Rows {
    Posts(Vec<search::PostHit>),
    Topics(Vec<topics::Topic>),
}

struct Found {
    total: i64,
    /// 返回的这一页（id 倒序，最多 [`PER_PAGE`] 条）的 id。
    ids: Vec<i64>,
    rows: Rows,
    /// 下一页从这个 id 之前继续；没有更多时为空。
    next: Option<i64>,
}

/// 从 `before` 往前按轮取候选，留下可见的，凑满一页或候选用完为止。
/// 某一轮凑满时游标停在本页最后一条；几轮都没凑满时停在最后看过的候选（下一页从那里继续，本页可能不满）。
async fn find_posts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    query: &str,
    forum_ids: &[i32],
    mut before: Option<i64>,
) -> ApiResult<Found> {
    let mut out: Vec<search::PostHit> = Vec::new();
    let mut total = 0;
    for limit in POST_ROUNDS {
        let hits = search::post_hits(&mut **tx, query, before, limit).await?;
        total = total.max(hits.total);
        let rows = search::post_rows(&mut **tx, &dedup(&hits), forum_ids).await?;
        let exhausted = (hits.ids.len() as i64) < limit;
        if let Some(next) = take_page(&mut out, rows, |p| p.id) {
            return Ok(found_posts(total, out, Some(next)));
        }
        before = hits.ids.iter().min().copied().or(before);
        if exhausted {
            return Ok(found_posts(total, out, None));
        }
    }
    Ok(found_posts(total, out, before))
}

impl Found {
    /// 「全部」里每种只留前几条（不翻页）。
    fn cut(mut self) -> Self {
        match &mut self.rows {
            Rows::Posts(rows) => rows.truncate(ALL_POSTS),
            Rows::Topics(rows) => rows.truncate(ALL_TOPICS),
        }
        self.next = None;
        self
    }
}

fn found_posts(total: i64, rows: Vec<search::PostHit>, next: Option<i64>) -> Found {
    Found {
        total,
        ids: rows.iter().map(|p| p.id).collect(),
        rows: Rows::Posts(rows),
        next,
    }
}

async fn find_topics(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    query: &str,
    forum_ids: &[i32],
    mut before: Option<i64>,
) -> ApiResult<Found> {
    let visible: HashSet<i32> = forum_ids.iter().copied().collect();
    let mut out: Vec<topics::Topic> = Vec::new();
    let mut total = 0;
    for limit in TOPIC_ROUNDS {
        let hits = search::topic_hits(&mut **tx, query, forum_ids, before, limit).await?;
        total = total.max(hits.total);
        let ids = dedup(&hits);
        let mut by_id: HashMap<i64, topics::Topic> = topics::by_ids(&mut **tx, &ids)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
        // Groonga 里的板块可能是移动前的旧值：按当前行再判断一次
        let rows: Vec<topics::Topic> = ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .filter(|t| t.flags & FLAG_HIDDEN == 0 && visible.contains(&t.forum_id))
            .collect();
        let exhausted = (hits.ids.len() as i64) < limit;
        if let Some(next) = take_page(&mut out, rows, |t| t.id) {
            return Ok(found_topics(total, out, Some(next)));
        }
        before = hits.ids.iter().min().copied().or(before);
        if exhausted {
            return Ok(found_topics(total, out, None));
        }
    }
    Ok(found_topics(total, out, before))
}

fn found_topics(total: i64, rows: Vec<topics::Topic>, next: Option<i64>) -> Found {
    Found {
        total,
        ids: rows.iter().map(|t| t.id).collect(),
        rows: Rows::Topics(rows),
        next,
    }
}

/// 去重并保持 id 倒序（同一楼层编辑前后的两个版本在清理前都在索引里）。
fn dedup(hits: &Hits) -> Vec<i64> {
    let mut seen = HashSet::new();
    hits.ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect()
}

/// 把这一轮的可见结果（id 倒序）追加到 `out`；凑满一页时返回最后一条的 id 作为游标。
fn take_page<T>(out: &mut Vec<T>, rows: Vec<T>, id: impl Fn(&T) -> i64) -> Option<i64> {
    for r in rows {
        out.push(r);
        if out.len() == PER_PAGE {
            return out.last().map(&id);
        }
    }
    None
}

/// 匹配用的字符折叠：全角 ASCII 转半角、转小写（与 NormalizerNFKC150 对拉丁字母与全角符号的处理一致）。
fn fold(c: char) -> char {
    let c = match c as u32 {
        0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        _ => c,
    };
    c.to_lowercase().next().unwrap_or(c)
}

/// 文字里各词的出现位置（字符下标的半开区间），从左到右、不重叠，同一位置优先长的词。
fn matches(text: &[char], terms: &[Vec<char>]) -> Vec<(usize, usize)> {
    let folded: Vec<char> = text.iter().copied().map(fold).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < folded.len() {
        let hit = terms
            .iter()
            .filter(|t| !t.is_empty() && folded[i..].starts_with(t))
            .map(Vec::len)
            .max();
        match hit {
            Some(n) => {
                out.push((i, i + n));
                i += n;
            }
            None => i += 1,
        }
    }
    out
}

fn fold_terms(terms: &[String]) -> Vec<Vec<char>> {
    terms
        .iter()
        .map(|t| t.chars().map(fold).collect())
        .collect()
}

/// 把 `text[from..to]` 切成高亮片段。
fn segments(text: &[char], spans: &[(usize, usize)], from: usize, to: usize) -> Vec<Highlight> {
    let mut out = Vec::new();
    let mut pos = from;
    let mut push = |a: usize, b: usize, hit: bool| {
        if a < b {
            out.push(Highlight {
                text: text[a..b].iter().collect(),
                hit,
            });
        }
    };
    for &(a, b) in spans.iter().filter(|&&(a, b)| a >= from && b <= to) {
        push(pos, a, false);
        push(a, b, true);
        pos = b;
    }
    push(pos, to, false);
    out
}

/// 标题高亮（整段）。
fn highlight(text: &str, terms: &[String]) -> Vec<Highlight> {
    let chars: Vec<char> = text.chars().collect();
    let spans = matches(&chars, &fold_terms(terms));
    segments(&chars, &spans, 0, chars.len())
}

/// 正文片段：第一处命中前留 [`SNIPPET_LEAD`] 字，共 [`SNIPPET_CHARS`] 字；没找到命中（如跨标点的 N 元组命中）时取开头。
fn snippet(text: &str, terms: &[String]) -> (Vec<Highlight>, bool, bool) {
    let chars: Vec<char> = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let spans = matches(&chars, &fold_terms(terms));
    let first = spans.first().map_or(0, |&(a, _)| a);
    let from = first
        .saturating_sub(SNIPPET_LEAD)
        .min(chars.len().saturating_sub(SNIPPET_CHARS));
    let to = (from + SNIPPET_CHARS).min(chars.len());
    // 截断处正好切在一个命中中间时不高亮它
    (
        segments(&chars, &spans, from, to),
        from > 0,
        to < chars.len(),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn terms(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| (*s).to_owned()).collect()
    }

    fn render(h: &[Highlight]) -> String {
        h.iter()
            .map(|s| {
                if s.hit {
                    format!("[{}]", s.text)
                } else {
                    s.text.clone()
                }
            })
            .collect()
    }

    #[test]
    fn highlights_case_and_width_insensitively() {
        let h = highlight(
            "出售 ChatGPT 账号，ＵＳＤＴ 付款",
            &terms(&["gpt", "usdt", "出售"]),
        );
        assert_eq!(render(&h), "[出售] Chat[GPT] 账号，[ＵＳＤＴ] 付款");
        // 原文按原样返回（不是折叠后的）
        assert!(h.iter().any(|s| s.hit && s.text == "ＵＳＤＴ"));
    }

    #[test]
    fn prefers_longer_overlapping_term_and_never_emits_markup() {
        let h = highlight("<b>担保交易</b>", &terms(&["担保", "担保交易"]));
        assert_eq!(render(&h), "<b>[担保交易]</b>");
        // 片段只是文字：尖括号原样留给前端当文本渲染
        assert_eq!(h[0].text, "<b>");
        assert!(!h[0].hit);
    }

    #[test]
    fn snippet_windows_around_first_hit() {
        let text = format!("{}关键词{}", "前".repeat(100), "后".repeat(300));
        let (h, start, end) = snippet(&text, &terms(&["关键词"]));
        assert!(start && end);
        let s = render(&h);
        assert_eq!(s.chars().count(), SNIPPET_CHARS + 2);
        assert!(s.starts_with(&"前".repeat(SNIPPET_LEAD)));
        assert!(s.contains("[关键词]"));
        // 短文与没命中时取开头
        let (h, start, end) = snippet("第一行\n第二行", &terms(&["无"]));
        assert_eq!(render(&h), "第一行 第二行");
        assert!(!start && !end);
        // 命中在末尾附近时窗口不越界
        let tail = format!("{}尾巴", "字".repeat(500));
        let (h, _, end) = snippet(&tail, &terms(&["尾巴"]));
        assert!(!end);
        assert!(render(&h).ends_with("[尾巴]"));
    }

    #[test]
    fn page_fills_and_reports_cursor() {
        let mut out = Vec::new();
        assert_eq!(take_page(&mut out, (1..=5).rev().collect(), |&x| x), None);
        let next = take_page(&mut out, (100..200).rev().collect(), |&x: &i64| x);
        assert_eq!(out.len(), PER_PAGE);
        assert_eq!(next, Some(185));
        assert_eq!(parse_cursor("185.20").unwrap(), (185, 20));
        assert!(parse_cursor("185").is_err());
        assert!(parse_cursor("x.1").is_err());
    }
}
