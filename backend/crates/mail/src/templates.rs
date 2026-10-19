//! 邮件模板：每种邮件只写一份，同时生成 HTML 与纯文本。

/// `(主题, HTML, 纯文本)`
pub type Rendered = (String, String, String);

fn layout(title: &str, body_html: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh-CN\"><body style=\"font-family:-apple-system,'PingFang SC','Noto Sans SC',sans-serif;font-size:16px;line-height:1.7;color:#1d1d1f\">\
         <h2 style=\"margin:0 0 16px\">{title}</h2>{body_html}\
         <p style=\"color:#86868b;font-size:13px\">官方永远不会向你索要验证码、恢复码，也不会要求私下转账。</p></body></html>"
    )
}

pub fn login_code(code: &str) -> Rendered {
    let subject = format!("你的登录验证码：{code}");
    let html = layout(
        "登录验证码",
        &format!(
            "<p>你的验证码是：</p><p style=\"font-size:32px;letter-spacing:6px;font-weight:600;font-variant-numeric:tabular-nums\">{code}</p>\
             <p>10 分钟内有效。如果不是你本人操作，请忽略这封邮件。</p>"
        ),
    );
    let text = format!(
        "你的登录验证码是：{code}\n10 分钟内有效。如果不是你本人操作，请忽略这封邮件。\n\n官方永远不会向你索要验证码、恢复码，也不会要求私下转账。"
    );
    (subject, html, text)
}

/// `at`：登录时间（UTC，已格式化）。
pub fn new_device(at: &str) -> Rendered {
    let subject = "你的账号在新设备上登录".to_owned();
    let html = layout(
        "新设备登录提醒",
        &format!(
            "<p>你的账号于 {at}（UTC）在一台新设备上登录。</p>\
             <p>如果不是你本人，请立即在「设置 → 设备管理」里踢出其他设备。资金操作仍需你的钱包 Passkey，单凭邮箱无法动用资金。</p>"
        ),
    );
    let text = format!(
        "你的账号于 {at}（UTC）在一台新设备上登录。\n如果不是你本人，请立即在「设置 → 设备管理」里踢出其他设备。资金操作仍需你的钱包 Passkey，单凭邮箱无法动用资金。"
    );
    (subject, html, text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn login_code_contains_code_in_both_parts() {
        let (subject, html, text) = super::login_code("123456");
        assert!(subject.contains("123456") && html.contains("123456") && text.contains("123456"));
    }
}
