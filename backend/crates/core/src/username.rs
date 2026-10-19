//! 用户名规则（plan 第 4 节「个人资料」）：2–20 个字符，只能是汉字、ASCII 字母、数字、下划线；
//! 不能是纯数字（`/users/{用户}` 里纯数字按用户 ID 解析）；保留名不区分大小写。
//! 只放 ASCII 字母而不放其他文字的字母，避免西里尔字母等形近字冒充别人。

use crate::ErrorCode;

pub const MIN_CHARS: usize = 2;
pub const MAX_CHARS: usize = 20;

/// 保留名（小写比较）：管理身份、系统账号与容易混淆的路由词。
const RESERVED: &[&str] = &[
    "admin",
    "administrator",
    "root",
    "system",
    "sysadmin",
    "moderator",
    "mod",
    "staff",
    "support",
    "service",
    "official",
    "akari",
    "arbiter",
    "arbitrator",
    "security",
    "api",
    "www",
    "help",
    "me",
    "null",
    "undefined",
    "anonymous",
    "guest",
    "deleted",
    "站长",
    "管理员",
    "超级管理员",
    "版主",
    "仲裁员",
    "客服",
    "官方",
    "系统",
    "系统消息",
    "匿名",
    "游客",
];

/// 汉字：CJK 统一表意文字基本区与扩展 A 区（不含兼容区、部首等形近符号）。
fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

/// 用户名里允许的字符。
pub fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || is_cjk(c)
}

/// 校验并整理（去首尾空白）用户名。
pub fn validate(name: &str) -> Result<String, ErrorCode> {
    let name = name.trim();
    let n = name.chars().count();
    if !(MIN_CHARS..=MAX_CHARS).contains(&n)
        || !name.chars().all(is_name_char)
        || name.chars().all(|c| c.is_ascii_digit())
        || RESERVED.contains(&name.to_lowercase().as_str())
    {
        return Err(ErrorCode::InvalidUsername);
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_cjk_letters_digits_underscore() {
        assert_eq!(validate(" 小明_abc9 ").unwrap(), "小明_abc9");
        assert!(validate("ab").is_ok());
        assert!(validate(&"字".repeat(20)).is_ok());
    }

    #[test]
    fn rejects_bad_names() {
        for bad in [
            "a",
            &"字".repeat(21),
            "a b",
            "a-b",
            "12345",
            "Admin",
            "站长",
            "аdmin", // 西里尔字母 а
            "名字😀",
            "ｆｕｌｌ", // 全角
        ] {
            assert_eq!(validate(bad), Err(ErrorCode::InvalidUsername), "{bad}");
        }
    }
}
