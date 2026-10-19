//! 帖子内容：HTML 过滤（ammonia，plan 第 11、14 节）、纯文本提取（供 PGroonga 搜索）、链接与 @提及识别；
//! 楼层评论（纯文本）的整理。
//!
//! 编辑器（Tiptap）提交 HTML 与 JSON 原稿；服务端只信任这里过滤后的 HTML。
//! 过滤在事务外完成，热帖抢楼时不占行锁（plan 第 8 节）。

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use ammonia::Builder;

/// 过滤前 HTML 最大字节数。
pub const MAX_HTML_BYTES: usize = 100 * 1024;
/// 标题最多字符数。
pub const MAX_TITLE_CHARS: usize = 120;
/// 楼层评论最多字符数（与 `floor_comments.body` 的约束一致）。
pub const MAX_COMMENT_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    #[error("content too long")]
    TooLong,
    #[error("content empty")]
    Empty,
    #[error("invalid title")]
    InvalidTitle,
}

/// 过滤后的帖子内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub html: String,
    pub plain_text: String,
    /// 链接与图片地址（`<a href>`、`<img src>`），原样保留相对地址。
    pub links: Vec<String>,
    /// 被 @ 的用户 ID（Tiptap mention 节点的 `data-id`），去重、保持顺序。
    pub mentions: Vec<i64>,
}

const BLOCK_TAGS: &[&str] = &[
    "p",
    "br",
    "li",
    "blockquote",
    "pre",
    "h2",
    "h3",
    "h4",
    "hr",
    "ul",
    "ol",
];

static SANITIZER: LazyLock<Builder<'static>> = LazyLock::new(|| {
    let tags: HashSet<&str> = [
        "p",
        "br",
        "strong",
        "b",
        "em",
        "i",
        "s",
        "u",
        "code",
        "pre",
        "blockquote",
        "ul",
        "ol",
        "li",
        "h2",
        "h3",
        "h4",
        "hr",
        "a",
        "img",
        "span",
    ]
    .into_iter()
    .collect();
    let attrs: HashMap<&str, HashSet<&str>> = [
        ("a", ["href"].into_iter().collect()),
        (
            "img",
            ["src", "alt", "width", "height"].into_iter().collect(),
        ),
        ("ol", ["start"].into_iter().collect()),
        ("code", ["class"].into_iter().collect()),
        (
            "span",
            ["data-type", "data-id", "data-label"].into_iter().collect(),
        ),
    ]
    .into_iter()
    .collect();
    let mut b = Builder::default();
    b.tags(tags)
        .tag_attributes(attrs)
        .generic_attributes(HashSet::new())
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .link_rel(Some("nofollow ugc noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(|element, attribute, value| {
            let keep = match (element, attribute) {
                // 图片只允许 https，防止混合内容与 data: 之类的地址
                ("img", "src") => value.starts_with("https://"),
                ("img", "width" | "height") | ("ol", "start") => {
                    value.len() <= 5 && value.bytes().all(|c| c.is_ascii_digit())
                }
                ("code", "class") => is_language_class(value),
                ("span", "data-type") => value == "mention",
                ("span", "data-id") => {
                    value.len() <= 20 && value.bytes().all(|c| c.is_ascii_digit())
                }
                _ => true,
            };
            keep.then(|| value.into())
        });
    b
});

fn is_language_class(v: &str) -> bool {
    v.strip_prefix("language-").is_some_and(|l| {
        (1..=20).contains(&l.len())
            && l.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'#' | b'-'))
    })
}

/// 过滤编辑器提交的 HTML。图片只保留 `image_hosts` 上的（本站图片 CDN / 存储桶，plan 第 12 节），
/// 其余图片连同标签一起删掉。结果没有任何可见文字、图片时返回 `Empty`。
pub fn clean(input: &str, image_hosts: &[String]) -> Result<Cleaned, ContentError> {
    if input.len() > MAX_HTML_BYTES {
        return Err(ContentError::TooLong);
    }
    let mut html = SANITIZER.clean(input).to_string();
    let scan = scan(&html, image_hosts);
    // 地址被过滤掉的 <img> 只剩空标签，去掉（从后往前删，前面的位置不变）
    for (start, end) in scan.dead_images.iter().rev() {
        html.replace_range(start..end, "");
    }
    let has_image = scan.images > 0;
    if scan.text.trim().is_empty() && !has_image {
        return Err(ContentError::Empty);
    }
    Ok(Cleaned {
        plain_text: scan.text.trim().to_owned(),
        links: scan.links,
        mentions: scan.mentions,
        html,
    })
}

/// 校验并整理标题：去首尾空白，1–120 个字符，不含控制字符。
pub fn clean_title(title: &str) -> Result<String, ContentError> {
    let t = title.trim();
    let n = t.chars().count();
    if n == 0 || n > MAX_TITLE_CHARS || t.chars().any(char::is_control) {
        return Err(ContentError::InvalidTitle);
    }
    Ok(t.to_owned())
}

/// 整理楼层评论（纯文本，不是 HTML）：换行、制表等控制字符换成空格，去掉零宽与双向文字控制字符
/// （防止倒序显示冒充别的内容），去首尾空白；结果 1–500 个字符。
pub fn clean_comment(text: &str) -> Result<String, ContentError> {
    // 先粗略挡住超大输入，不为它分配
    if text.len() > MAX_COMMENT_CHARS * 8 {
        return Err(ContentError::TooLong);
    }
    let body: String = text
        .chars()
        .filter(|&c| !is_invisible_format(c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let body = body.trim();
    if body.is_empty() {
        return Err(ContentError::Empty);
    }
    if body.chars().count() > MAX_COMMENT_CHARS {
        return Err(ContentError::TooLong);
    }
    Ok(body.to_owned())
}

/// 零宽字符、双向文字嵌入 / 覆盖 / 隔离控制符与 BOM。
fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// 是否含站外链接：绝对地址（含 `mailto:`）的主机既不在 `internal_hosts`（本站）也不在 `image_hosts`
/// （本站图片的公开主机，即 `clean` 用的那份）内即为站外；相对地址算站内。用户贴自己上传的图片不算外链。
pub fn has_external_link(
    links: &[String],
    internal_hosts: &[String],
    image_hosts: &[String],
) -> bool {
    links.iter().any(|l| match host_of(l) {
        Some(h) => !internal_hosts
            .iter()
            .chain(image_hosts)
            .any(|i| i.eq_ignore_ascii_case(h)),
        None => l.contains(':'), // mailto: 等其他协议
    })
}

/// `https://` 且主机在本站图片主机列表里（ammonia 已保证只剩 https 地址）。
fn is_own_image(src: &str, image_hosts: &[String]) -> bool {
    src.starts_with("https://")
        && host_of(src).is_some_and(|h| image_hosts.iter().any(|i| i.eq_ignore_ascii_case(h)))
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("//"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit('@').next().unwrap_or(authority);
    Some(host.split(':').next().unwrap_or(host))
}

struct Scan {
    text: String,
    links: Vec<String>,
    mentions: Vec<i64>,
    images: usize,
    /// 没有 src 或不是本站图片的 <img> 标签在 HTML 中的字节范围。
    dead_images: Vec<(usize, usize)>,
}

/// 扫描 ammonia 输出（结构已规范：属性一律双引号、文本里的 `<` 已转义），提取纯文本、链接与提及。
fn scan(html: &str, image_hosts: &[String]) -> Scan {
    let mut out = Scan {
        text: String::with_capacity(html.len() / 2),
        links: Vec::new(),
        mentions: Vec::new(),
        images: 0,
        dead_images: Vec::new(),
    };
    let mut pos = 0;
    while let Some(lt) = html[pos..].find('<').map(|i| pos + i) {
        push_text(&mut out.text, &html[pos..lt]);
        let end = lt + 1 + tag_end(&html[lt + 1..]);
        if !handle_tag(&html[lt + 1..end], image_hosts, &mut out) {
            out.dead_images.push((lt, (end + 1).min(html.len())));
        }
        pos = (end + 1).min(html.len());
    }
    let rest = &html[pos..];
    push_text(&mut out.text, rest);
    out
}

/// 找到标签结束的 `>`，跳过双引号内的内容（属性值里的 `>` 不转义）。
fn tag_end(s: &str) -> usize {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '>' if !quoted => return i,
            _ => {}
        }
    }
    s.len()
}

/// 处理一个标签；返回 `false` 表示这是一个应删除的图片（无地址或不是本站图片）。
fn handle_tag(tag: &str, image_hosts: &[String], out: &mut Scan) -> bool {
    let closing = tag.starts_with('/');
    let body = tag.trim_start_matches('/');
    let name_end = body
        .find(|c: char| c.is_ascii_whitespace() || c == '/')
        .unwrap_or(body.len());
    let name = &body[..name_end];
    if BLOCK_TAGS.contains(&name) && (closing || matches!(name, "br" | "hr")) {
        out.text.push('\n');
    }
    if closing {
        return true;
    }
    let attrs = parse_attrs(&body[name_end..]);
    let get = |k: &str| attrs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
    match name {
        "a" => out.links.extend(get("href")),
        "img" => match get("src") {
            Some(src) if is_own_image(&src, image_hosts) => {
                out.images += 1;
                out.links.push(src);
            }
            _ => return false,
        },
        "span" if get("data-type").as_deref() == Some("mention") => {
            if let Some(id) = get("data-id").and_then(|v| v.parse::<i64>().ok())
                && !out.mentions.contains(&id)
            {
                out.mentions.push(id);
            }
        }
        _ => {}
    }
    true
}

fn parse_attrs(s: &str) -> Vec<(&str, String)> {
    let mut attrs = Vec::new();
    let mut rest = s;
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq].trim();
        let after = &rest[eq + 2..];
        let close = after.find('"').unwrap_or(after.len());
        attrs.push((name, decode(&after[..close])));
        rest = after.get(close + 1..).unwrap_or("");
    }
    attrs
}

fn push_text(buf: &mut String, s: &str) {
    if !s.is_empty() {
        buf.push_str(&decode(s));
    }
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_t(input: &str) -> Result<Cleaned, ContentError> {
        clean(input, &["cdn.test".to_owned()])
    }

    #[test]
    fn strips_scripts_and_handlers() {
        let c =
            clean_t(r#"<p onclick="x()">hi<script>alert(1)</script></p><img src="javascript:1">"#)
                .unwrap();
        assert_eq!(c.html, "<p>hi</p>");
        assert_eq!(c.plain_text, "hi");
    }

    #[test]
    fn links_get_rel_and_bad_schemes_dropped() {
        let c = clean_t(
            r#"<p><a href="https://ex.com/a">x</a><a href="javascript:alert(1)">y</a></p>"#,
        )
        .unwrap();
        assert!(c.html.contains(r#"rel="nofollow ugc noopener noreferrer""#));
        assert!(!c.html.contains("javascript"));
        assert_eq!(c.links, vec!["https://ex.com/a".to_owned()]);
    }

    #[test]
    fn images_https_only() {
        let c = clean_t(
            r#"<p><img src="http://a/x.png"><img src="https://cdn.test/y.webp" alt="y"></p>"#,
        )
        .unwrap();
        assert_eq!(c.links, vec!["https://cdn.test/y.webp".to_owned()]);
        assert!(!c.html.contains("http://a"));
    }

    #[test]
    fn images_only_from_own_hosts() {
        let c = clean_t(concat!(
            r#"<p>x<img src="https://evil.test/a.webp"><img src="https://cdn.test@evil.test/b.webp">"#,
            r#"<img src="https://CDN.test/i/c.webp"></p>"#
        ))
        .unwrap();
        assert_eq!(c.html, r#"<p>x<img src="https://CDN.test/i/c.webp"></p>"#);
        assert_eq!(c.links, vec!["https://CDN.test/i/c.webp".to_owned()]);
        assert_eq!(
            clean(r#"<p><img src="https://cdn.test/y.webp"></p>"#, &[]),
            Err(ContentError::Empty)
        );
    }

    #[test]
    fn image_only_post_is_not_empty() {
        assert!(clean_t(r#"<p><img src="https://cdn.test/y.webp"></p>"#).is_ok());
        assert_eq!(clean_t("<p> </p><p></p>"), Err(ContentError::Empty));
        assert_eq!(
            clean_t(r#"<p><img src="http://x/y.png"></p>"#),
            Err(ContentError::Empty)
        );
    }

    #[test]
    fn plain_text_breaks_blocks_and_decodes_entities() {
        let c = clean_t("<p>a &amp; b &lt;c&gt;</p><p>第二段</p><ul><li>一</li><li>二</li></ul>")
            .unwrap();
        assert_eq!(c.plain_text, "a & b <c>\n第二段\n一\n二");
    }

    #[test]
    fn quoted_gt_in_attribute_does_not_break_scan() {
        let c = clean_t(r#"<p><a href="https://ex.com/?q=a>b">link</a> after</p>"#).unwrap();
        assert_eq!(c.plain_text, "link after");
    }

    #[test]
    fn code_language_class_and_mentions() {
        let c = clean_t(concat!(
            r#"<pre><code class="language-rust">fn main() {}</code></pre>"#,
            r#"<pre><code class="evil">x</code></pre>"#,
            r#"<p><span data-type="mention" data-id="42" data-label="lam">@lam</span>"#,
            r#"<span data-type="mention" data-id="42">@lam</span>"#,
            r#"<span data-type="mention" data-id="x1">@bad</span></p>"#,
        ))
        .unwrap();
        assert!(c.html.contains(r#"class="language-rust""#));
        assert!(!c.html.contains("evil"));
        assert_eq!(c.mentions, vec![42]);
    }

    #[test]
    fn external_link_detection() {
        let internal = vec!["forum.test".to_owned()];
        let images = vec!["cdn.test".to_owned()];
        let ext = |l: &str| has_external_link(&[l.to_owned()], &internal, &images);
        assert!(!ext("/t/1"));
        // 本站图片：clean 把它们也记进 links
        let own = clean(r#"<p><img src="https://cdn.test/i/a.webp"></p>"#, &images).unwrap();
        assert_eq!(own.links, ["https://cdn.test/i/a.webp"]);
        assert!(!has_external_link(&own.links, &internal, &images));
        assert!(has_external_link(&own.links, &internal, &[]));
        assert!(ext("https://cdn.test.evil.test/a.webp"));
        assert!(!ext("https://forum.test/t/1"));
        assert!(!ext("https://FORUM.test:443/x"));
        assert!(ext("https://evil.test/"));
        assert!(ext("https://forum.test@evil.test/"));
        assert!(ext("mailto:a@b.c"));
    }

    #[test]
    fn title_rules() {
        assert_eq!(clean_title("  标题 ").unwrap(), "标题");
        assert_eq!(clean_title(""), Err(ContentError::InvalidTitle));
        assert_eq!(clean_title("a\u{7}b"), Err(ContentError::InvalidTitle));
        assert!(clean_title(&"字".repeat(120)).is_ok());
        assert!(clean_title(&"字".repeat(121)).is_err());
    }

    #[test]
    fn comment_rules() {
        assert_eq!(
            clean_comment("  第一行\r\n第二行\t<b>x</b> ").unwrap(),
            "第一行  第二行 <b>x</b>"
        );
        assert_eq!(
            clean_comment("a\u{202E}gpj.exe\u{200B}").unwrap(),
            "agpj.exe"
        );
        assert_eq!(clean_comment(" \n\u{200B} "), Err(ContentError::Empty));
        assert!(clean_comment(&"字".repeat(MAX_COMMENT_CHARS)).is_ok());
        assert_eq!(
            clean_comment(&"字".repeat(MAX_COMMENT_CHARS + 1)),
            Err(ContentError::TooLong)
        );
        assert_eq!(
            clean_comment(&"a".repeat(MAX_COMMENT_CHARS * 8 + 1)),
            Err(ContentError::TooLong)
        );
    }

    #[test]
    fn too_long() {
        assert_eq!(
            clean_t(&"a".repeat(MAX_HTML_BYTES + 1)),
            Err(ContentError::TooLong)
        );
    }
}
