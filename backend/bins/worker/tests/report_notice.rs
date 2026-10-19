//! 举报结案通知的渲染（payload 由 forum-api 结案时写入发件箱，见 `forum_db::reports::resolve`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::outbox::Message;
use forum_worker::{Delivery, render};
use serde_json::json;

#[test]
fn report_resolution_becomes_a_system_notice_without_actor() {
    let msg = Message {
        id: 1,
        kind: forum_db::reports::NOTIFY_RESOLVED.into(),
        payload: json!({"to": 3, "outcome": "actioned", "topic_id": 9, "floor": 2}),
        attempts: 0,
    };
    let Delivery::InSite(site) = render(&msg).unwrap() else {
        panic!("expected an in-site notice");
    };
    let n = &site.notifications[0];
    // 不带处理人，也没有处理方式
    assert_eq!((n.user_id, n.kind, n.actor_id), (3, "system", None));
    assert_eq!(
        n.payload,
        json!({"event": "report_resolved", "outcome": "actioned", "topic_id": 9, "floor": 2})
    );
}
