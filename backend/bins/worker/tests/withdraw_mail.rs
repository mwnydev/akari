//! 提币地址邮件的渲染（payload 由 forum-api 写入发件箱，见 `routes::withdraw`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::outbox::Message;
use forum_worker::{Delivery, render};
use serde_json::json;

#[test]
fn withdraw_address_added_is_emailed_with_its_activation_time() {
    let msg = Message {
        id: 7,
        kind: "email.withdraw_address_added".into(),
        payload: json!({
            "to": "a@example.com",
            "address": "0x1111111111111111111111111111111111111111",
            "active_at": "2026-10-20 08:00 UTC",
            "immediate": false,
        }),
        attempts: 0,
    };
    let Delivery::Email(email) = render(&msg).unwrap() else {
        panic!("expected an email");
    };
    assert_eq!(email.to, "a@example.com");
    assert_eq!(
        email.subject,
        "提币地址已添加，将于 2026-10-20 08:00 UTC 生效"
    );
    assert!(
        email
            .text
            .contains("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(email.idempotency_key, "outbox-7");

    // 站内提醒成为 `system` 通知，`event` 标明来源。
    let notice = Message {
        kind: "notify.withdraw_address_added".into(),
        payload: json!({
            "to": 3,
            "withdraw_address_id": 5,
            "address": "0x1111111111111111111111111111111111111111",
            "active_at": "2026-10-20T08:00:00Z",
        }),
        ..msg
    };
    let Delivery::InSite(site) = render(&notice).unwrap() else {
        panic!("expected an in-site notice");
    };
    assert_eq!(site.notifications.len(), 1);
    let n = &site.notifications[0];
    assert_eq!((n.user_id, n.kind, n.actor_id), (3, "system", None));
    assert_eq!(n.payload["event"], "withdraw_address_added");
    assert_eq!(n.payload["withdraw_address_id"], 5);
}
