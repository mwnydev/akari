//! 邮箱归一化与基本格式校验。

use crate::ErrorCode;

const MAX_LEN: usize = 254;

/// 去首尾空白并转小写，再做基本格式校验（一个 `@`、两侧非空、域名含点、无空白）。
/// 不尝试完整实现 RFC 5322：真正的校验是验证码能否送达。
pub fn normalize_email(raw: &str) -> Result<String, ErrorCode> {
    let email = raw.trim().to_lowercase();
    if email.len() > MAX_LEN || email.chars().any(char::is_whitespace) {
        return Err(ErrorCode::InvalidEmail);
    }
    let (local, domain) = email.split_once('@').ok_or(ErrorCode::InvalidEmail)?;
    let domain_ok = domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@');
    if local.is_empty() || !domain_ok {
        return Err(ErrorCode::InvalidEmail);
    }
    Ok(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_validates() {
        assert_eq!(
            normalize_email("  Foo@Example.COM ").unwrap(),
            "foo@example.com"
        );
        for bad in [
            "",
            "foo",
            "@a.com",
            "a@",
            "a@b",
            "a@.com",
            "a@com.",
            "a b@c.com",
            "a@b@c.com",
        ] {
            assert_eq!(normalize_email(bad), Err(ErrorCode::InvalidEmail), "{bad}");
        }
        let long = format!("{}@example.com", "a".repeat(250));
        assert_eq!(normalize_email(&long), Err(ErrorCode::InvalidEmail));
    }
}
