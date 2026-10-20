//! 邮箱归一化与基本格式校验；邮件里的时间格式。

use time::OffsetDateTime;

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

/// 邮件里给人看的时间：`2026-10-20 08:00 UTC`。
pub fn display_time(t: OffsetDateTime) -> String {
    let t = t.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute()
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn display_time_is_minutes_in_utc() {
        let t = OffsetDateTime::from_unix_timestamp(1_792_000_000).unwrap();
        assert_eq!(display_time(t), "2026-10-14 17:46 UTC");
    }

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
