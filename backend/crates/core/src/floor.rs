//! 楼层下的朋友圈式社交区（设计稿 v2）：点赞与两层评论（楼层 → 平铺评论，回复评论只是记下被回复的人）。
//! REST 响应与 WebSocket 推送用同一份 JSON（[`FloorComment`]）。

use serde::Serialize;
use time::OffsetDateTime;

use crate::username;

/// 主题页每楼随楼层一起返回的评论条数（其余点「展开」再分页取）。
pub const PREVIEW_COMMENTS: i64 = 5;
/// 主题页每楼列出的点赞人数（最近的几位，其余显示「等 N 人」）。
pub const PREVIEW_LIKERS: i64 = 5;
/// 一条评论最多通知几个被 @ 的人。
pub const MAX_COMMENT_MENTIONS: usize = 5;

/// 楼层下的一条评论。`body` 是纯文本（服务端已去掉控制字符），客户端必须按文本渲染，不能当 HTML。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FloorComment {
    pub id: i64,
    pub post_id: i64,
    pub author_id: i64,
    /// 回复某条评论时，那条评论的作者。
    pub reply_to_id: Option<i64>,
    pub body: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// 纯文本里的 `@用户名`（按出现顺序去重，小写，最多 [`MAX_COMMENT_MENTIONS`] 个）。
/// 用户名按 [`username`] 的字符规则取最长的一段；`@` 前紧挨着用户名字符（如邮箱）的不算。
pub fn mention_names(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let after_name_char = prev.is_some_and(username::is_name_char);
        prev = Some(c);
        if c != '@' || after_name_char {
            continue;
        }
        let rest = &text[i + 1..];
        let end = rest
            .char_indices()
            .find(|&(_, c)| !username::is_name_char(c))
            .map_or(rest.len(), |(j, _)| j);
        let name = &rest[..end];
        let n = name.chars().count();
        if (username::MIN_CHARS..=username::MAX_CHARS).contains(&n) {
            let lower = name.to_lowercase();
            if !out.contains(&lower) {
                out.push(lower);
                if out.len() == MAX_COMMENT_MENTIONS {
                    break;
                }
            }
        }
        // 跳过这个用户名，`@a@b` 里的第二个 `@` 紧挨着用户名字符，不算。
        while chars.peek().is_some_and(|&(j, _)| j <= i + end) {
            prev = chars.next().map(|(_, c)| c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_follow_username_rules() {
        assert_eq!(
            mention_names("@小林 你看 @Momo_1，还有@小林"),
            ["小林", "momo_1"]
        );
        // 邮箱、单字、超长都不算
        assert!(mention_names("mail a@bc.com @x").is_empty());
        assert!(mention_names(&format!("@{}", "a".repeat(21))).is_empty());
        assert_eq!(mention_names("@ab@cd"), ["ab"]);
        assert_eq!(mention_names("（@张三）"), ["张三"]);
        let many: String = (0..8).map(|i| format!("@user{i} ")).collect();
        assert_eq!(mention_names(&many).len(), MAX_COMMENT_MENTIONS);
    }
}
