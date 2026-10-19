//! 全文搜索（PGroonga，plan 第 8 节）。先在 Groonga 里匹配并按 id 倒序取前 N 个，再按主键回表检查可见性，
//! 原因见迁移 `20261212000001_search.sql`。Groonga 查询串由调用方用 [`groonga_query`] 生成。

use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;

/// 一个查询最多几个词（多出的忽略）。
pub const MAX_TERMS: usize = 5;
/// 查询串最多多少个字符（多出的截掉）。
pub const MAX_QUERY_CHARS: usize = 64;

/// 把用户输入变成 Groonga 查询语法：按空白切成最多 [`MAX_TERMS`] 个词，每个词加双引号作为短语（引号内只有 `"` 与 `\`
/// 需要转义），词之间是「与」。用户写不出 `OR`、`-`、括号、`列名:` 之类的运算符。去掉控制字符；没有词时返回 `None`。
pub fn groonga_query(input: &str) -> Option<(String, Vec<String>)> {
    let cleaned: String = input
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_QUERY_CHARS)
        .collect();
    let terms: Vec<String> = cleaned
        .split_whitespace()
        .take(MAX_TERMS)
        .map(str::to_owned)
        .collect();
    if terms.is_empty() {
        return None;
    }
    let query = terms
        .iter()
        .map(|t| format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(" ");
    Some((query, terms))
}

/// Groonga 一次 select 的结果：命中总数与按 id 倒序的前 N 个 id（可能有重复，见查询文件的说明）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hits {
    pub total: i64,
    pub ids: Vec<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("groonga: {0}")]
    Groonga(String),
}

/// 解析 `[[返回码, …], [[[总数], [[列名, 类型]], [id], …]]]`。返回码非 0 时为错误（附 Groonga 的错误信息）。
fn parse_hits(v: &Value) -> Result<Hits, SearchError> {
    if v[0][0].as_i64() != Some(0) {
        return Err(SearchError::Groonga(v[0].to_string()));
    }
    let set = v[1][0]
        .as_array()
        .ok_or_else(|| SearchError::Groonga(format!("unexpected result: {v}")))?;
    let total = set.first().and_then(|c| c[0].as_i64()).unwrap_or(0);
    let ids = set.iter().skip(2).filter_map(|r| r[0].as_i64()).collect();
    Ok(Hits { total, ids })
}

/// 楼层：匹配 `query` 且 id 小于 `before`（为空表示从最新开始）的前 `limit` 个。
pub async fn post_hits(
    db: impl PgExecutor<'_>,
    query: &str,
    before: Option<i64>,
    limit: i64,
) -> Result<Hits, SearchError> {
    let filter = before.map(|id| format!("id < {id}")).unwrap_or_default();
    let row = sqlx::query_file!("queries/search_groonga_posts.sql", query, filter, limit)
        .fetch_one(db)
        .await?;
    parse_hits(&row.result)
}

/// 主题标题：只在 `forum_ids` 这些板块里（为空时没有结果），id 小于 `before`。
pub async fn topic_hits(
    db: impl PgExecutor<'_>,
    query: &str,
    forum_ids: &[i32],
    before: Option<i64>,
    limit: i64,
) -> Result<Hits, SearchError> {
    if forum_ids.is_empty() {
        return Ok(Hits {
            total: 0,
            ids: Vec::new(),
        });
    }
    let ids: Vec<String> = forum_ids.iter().map(i32::to_string).collect();
    let mut filter = format!("in_values(forum_id, {})", ids.join(", "));
    if let Some(id) = before {
        filter.push_str(&format!(" && id < {id}"));
    }
    let row = sqlx::query_file!("queries/search_groonga_topics.sql", query, filter, limit)
        .fetch_one(db)
        .await?;
    parse_hits(&row.result)
}

/// 搜索结果里的一楼（回表后、已按可见性过滤）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostHit {
    pub id: i64,
    pub topic_id: i64,
    pub floor: i32,
    pub author_id: i64,
    pub plain_text: String,
    pub created_at: OffsetDateTime,
    pub title: String,
    pub forum_id: i32,
}

/// 按 id 回表：未删除、主题未隐藏、在 `forum_ids` 里的楼层，id 倒序。
pub async fn post_rows(
    db: impl PgExecutor<'_>,
    ids: &[i64],
    forum_ids: &[i32],
) -> Result<Vec<PostHit>, sqlx::Error> {
    if ids.is_empty() || forum_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(PostHit, "queries/search_posts_rows.sql", ids, forum_ids)
        .fetch_all(db)
        .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn query_quotes_every_term_and_escapes() {
        let (q, terms) = groonga_query("  出售 \t Netflix  ").unwrap();
        assert_eq!(q, r#""出售" "Netflix""#);
        assert_eq!(terms, ["出售", "Netflix"]);
        // 运算符与列名语法都在引号里，按普通文字处理
        let (q, _) = groonga_query(r#"a" OR plain_text:@x \ (-z)"#).unwrap();
        assert_eq!(q, r#""a\"" "OR" "plain_text:@x" "\\" "(-z)""#);
        assert!(groonga_query(" \n\t ").is_none());
        assert!(groonga_query("").is_none());
    }

    #[test]
    fn query_limits_terms_length_and_control_chars() {
        let (_, terms) = groonga_query("a b c d e f g").unwrap();
        assert_eq!(terms.len(), MAX_TERMS);
        let (_, terms) = groonga_query(&"长".repeat(200)).unwrap();
        assert_eq!(terms[0].chars().count(), MAX_QUERY_CHARS);
        let (q, _) = groonga_query("a\u{0}b\u{1b}c").unwrap();
        assert_eq!(q, r#""a" "b" "c""#);
    }

    #[test]
    fn parses_groonga_select() {
        let v = json!([[0, 1.0, 0.01], [[[3], [["id", "Int64"]], [9], [7], [7]]]]);
        assert_eq!(
            parse_hits(&v).unwrap(),
            Hits {
                total: 3,
                ids: vec![9, 7, 7]
            }
        );
        let empty = json!([[0, 1.0, 0.01], [[[0], [["id", "Int64"]]]]]);
        assert_eq!(parse_hits(&empty).unwrap().ids, Vec::<i64>::new());
        let err = json!([[-22, 1.0, 0.0, "syntax error"], []]);
        assert!(matches!(parse_hits(&err), Err(SearchError::Groonga(_))));
    }
}
