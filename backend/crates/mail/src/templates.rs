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

/// 开通交易前的邮箱再验证码（plan 第 5 节）。正文写明用途，被盗号者触发时用户能察觉。
pub fn wallet_code(code: &str) -> Rendered {
    let subject = format!("开通交易验证码：{code}");
    let html = layout(
        "开通交易验证码",
        &format!(
            "<p>你正在开通交易（创建钱包 Passkey）。验证码是：</p><p style=\"font-size:32px;letter-spacing:6px;font-weight:600;font-variant-numeric:tabular-nums\">{code}</p>\
             <p>10 分钟内有效。如果不是你本人操作，说明有人登录了你的账号：请勿把验证码告诉任何人，并立即在「设置 → 设备管理」里踢出其他设备。</p>"
        ),
    );
    let text = format!(
        "你正在开通交易（创建钱包 Passkey）。验证码是：{code}\n10 分钟内有效。如果不是你本人操作，说明有人登录了你的账号：请勿把验证码告诉任何人，并立即在「设置 → 设备管理」里踢出其他设备。\n\n官方永远不会向你索要验证码、恢复码，也不会要求私下转账。"
    );
    (subject, html, text)
}

/// 站长绑定第一个后台 Passkey 前的邮箱验证码（ADR 0013）。正文写明用途，被盗号者触发时站长能察觉。
pub fn admin_code(code: &str) -> Rendered {
    let subject = format!("绑定后台 Passkey 验证码：{code}");
    let html = layout(
        "绑定后台 Passkey",
        &format!(
            "<p>你正在为站长后台绑定第一个 Passkey。验证码是：</p><p style=\"font-size:32px;letter-spacing:6px;font-weight:600;font-variant-numeric:tabular-nums\">{code}</p>\
             <p>10 分钟内有效。如果不是你本人操作，说明有人登录了你的论坛账号：请勿把验证码告诉任何人，立即踢出其他设备并检查邮箱安全。</p>"
        ),
    );
    let text = format!(
        "你正在为站长后台绑定第一个 Passkey。验证码是：{code}\n10 分钟内有效。如果不是你本人操作，说明有人登录了你的论坛账号：请勿把验证码告诉任何人，立即踢出其他设备并检查邮箱安全。"
    );
    (subject, html, text)
}

/// 添加了提币地址（plan 第 5 节「提币地址冷却」）。`address` 是完整地址，`active_at` 是已格式化的生效时间；
/// 第一个地址 `immediate` 为真。冷却期内本人可以撤销，正文写明不是本人操作时怎么办。
pub fn withdraw_address_added(address: &str, active_at: &str, immediate: bool) -> Rendered {
    let when = if immediate {
        "已立即生效".to_owned()
    } else {
        format!("将于 {active_at} 生效")
    };
    let subject = format!("提币地址已添加，{when}");
    let advice = if immediate {
        "如果不是你本人操作，请立即在钱包页「提币地址」里撤销，并在「设置 → 设备管理」里踢出其他设备。"
    } else {
        "生效前可以随时撤销。如果不是你本人操作，请立即在钱包页「提币地址」里撤销，并在「设置 → 设备管理」里踢出其他设备。"
    };
    let html = layout(
        "提币地址已添加",
        &format!(
            "<p>你的账号添加了提币地址：</p><p style=\"font-family:ui-monospace,monospace;word-break:break-all\">{address}</p>\
             <p>{when}。{advice}</p>"
        ),
    );
    let text = format!(
        "你的账号添加了提币地址：{address}\n{when}。{advice}\n\n官方永远不会向你索要验证码、恢复码，也不会要求私下转账。"
    );
    (subject, html, text)
}

/// 论坛裁决 Passkey 首次登记前的邮箱验证码（ADR 0013 第 4 条）。正文写明用途：被盗号者触发时本人能察觉。
pub fn ruling_passkey_code(code: &str) -> Rendered {
    let subject = format!("登记裁决 Passkey 验证码：{code}");
    let body = "你正在登记用于签署仲裁裁决的 Passkey。";
    let advice = "10 分钟内有效。如果不是你本人操作，说明有人登录了你的管理账号：请勿把验证码告诉任何人，并立即联系站长。";
    let html = layout(
        "登记裁决 Passkey",
        &format!(
            "<p>{body}验证码是：</p><p style=\"font-size:32px;letter-spacing:6px;font-weight:600;font-variant-numeric:tabular-nums\">{code}</p>\
             <p>{advice}</p>"
        ),
    );
    let text = format!(
        "{body}验证码是：{code}\n{advice}\n\n官方永远不会向你索要验证码、恢复码，也不会要求私下转账。"
    );
    (subject, html, text)
}

/// 拍卖中标（含递补）或付款提醒（`reminder`）。`deadline` 是已格式化的付款截止时间。只写拍卖编号，不带帖子内容
/// （用户输入）；正文写明逾期的后果。
pub fn auction_payment(auction_id: i64, deadline: &str, reminder: bool) -> Rendered {
    let (subject, title, lead) = if reminder {
        (
            format!("拍卖 #{auction_id} 付款截止时间快到了"),
            "拍卖付款提醒",
            format!("你在拍卖 #{auction_id} 中标，还没有付款。"),
        )
    } else {
        (
            format!("你已中标拍卖 #{auction_id}，请在 {deadline} 前付款"),
            "拍卖中标",
            format!("你已中标拍卖 #{auction_id}。"),
        )
    };
    let advice = "逾期未付款会记一次违约（在信誉里公开显示），并在一段时间内不能参与拍卖。";
    let action = "请登录论坛，在拍卖帖里付款建立担保订单。";
    let html = layout(
        title,
        &format!("<p>{lead}请在 <strong>{deadline}</strong> 前付款。</p><p>{action}{advice}</p>"),
    );
    let text = format!(
        "{lead}请在 {deadline} 前付款。\n{action}{advice}\n\n官方永远不会向你索要验证码、恢复码，也不会要求私下转账。"
    );
    (subject, html, text)
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// 仲裁案件提醒（发给站长）：`title` 为事件说明，`lines` 为逐行明细（裁决理由等用户输入的内容会转义）。
pub fn arbitration_alert(case_id: i64, title: &str, lines: &[String]) -> Rendered {
    let subject = format!("仲裁案件 #{case_id}：{title}");
    let items: String = lines
        .iter()
        .map(|l| format!("<li style=\"white-space:pre-wrap\">{}</li>", escape(l)))
        .collect();
    let html = layout(
        &escape(&subject),
        &format!("<ul>{items}</ul><p>请登录论坛仲裁页面查看完整案情与证据。</p>"),
    );
    let text = format!(
        "{subject}\n\n{}\n\n请登录论坛仲裁页面查看完整案情与证据。",
        lines.join("\n")
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

    #[test]
    fn wallet_code_names_its_purpose() {
        let (subject, html, text) = super::wallet_code("654321");
        assert!(subject.contains("654321") && html.contains("654321") && text.contains("654321"));
        assert!(subject.contains("开通交易") && text.contains("不是你本人"));
    }

    #[test]
    fn arbitration_alert_escapes_user_text() {
        let (subject, html, text) = super::arbitration_alert(
            7,
            "裁决已公示",
            &["理由：<script>x</script> & 证据".to_owned()],
        );
        assert!(subject.contains("#7"));
        assert!(
            html.contains("&lt;script&gt;x&lt;/script&gt; &amp; 证据")
                && !html.contains("<script>")
        );
        assert!(text.contains("<script>x</script>"));
        let (subject, _, text) = super::ruling_passkey_code("112233");
        assert!(subject.contains("112233") && text.contains("裁决"));
    }

    #[test]
    fn auction_payment_states_the_deadline() {
        let (subject, html, text) = super::auction_payment(42, "2026-10-20 08:00 UTC", false);
        assert!(subject.contains("#42") && subject.contains("2026-10-20 08:00 UTC"));
        assert!(html.contains("2026-10-20 08:00 UTC") && text.contains("违约"));
        let (subject, _, text) = super::auction_payment(42, "2026-10-20 08:00 UTC", true);
        assert!(subject.contains("快到了") && text.contains("2026-10-20 08:00 UTC"));
    }

    #[test]
    fn withdraw_address_added_states_when_it_takes_effect() {
        let addr = "0x1111111111111111111111111111111111111111";
        let (subject, html, text) =
            super::withdraw_address_added(addr, "2026-10-20 08:00 UTC", false);
        assert!(subject.contains("将于 2026-10-20 08:00 UTC 生效"));
        assert!(html.contains(addr) && text.contains(addr));
        assert!(text.contains("撤销"));
        let (subject, _, text) = super::withdraw_address_added(addr, "ignored", true);
        assert!(subject.contains("立即生效") && !text.contains("ignored"));
    }
}
