//! 订单状态变化写进聊天（plan 第 7 节）：订单聊天的系统消息（付款、发货、开始服务、和解提议、争议、结束、回滚修正），
//! 私信里报价卡片的「已付款」，以及建单前谈判记录的关联。都与订单行的改写在同一事务里。
//!
//! 写完的消息另记一条发件箱（`push.order_message` / `push.message`），worker 在事务提交后经 Valkey 推送。

use forum_db::chain::{ChatBrief, OfferFill};
use forum_db::messages::{self, NewMessage};
use forum_db::order_chat::{self, NewOrderMessage};
use forum_db::trade::OfferStatus;
use forum_db::{Postgres, Transaction, outbox};
use serde_json::json;

use crate::engine::Cause;

/// 发件箱：推送订单聊天新消息 `{id}` / 私信新消息 `{id}`。
pub const PUSH_ORDER_MESSAGE: &str = "push.order_message";
pub const PUSH_MESSAGE: &str = "push.message";
/// 发件箱：报价卡片事件通知（与 forum-api 写的同一种）。
pub const NOTIFY_OFFER: &str = "notify.offer";

const STATE_NONE: i16 = 0;
const STATE_RELEASED: i16 = 3;
const STATE_REFUNDED: i16 = 4;
const STATE_SETTLED: i16 = 5;

fn side(addr: &[u8], buyer: &[u8]) -> &'static str {
    if addr == buyer { "买家" } else { "卖家" }
}

/// 订单从 `prev`（库里旧行；新订单为空）变成 `now` 时要写的系统消息。只看链上事件与回滚，确认级别变化不写。
pub fn system_messages(prev: Option<&ChatBrief>, now: &ChatBrief, cause: Cause) -> Vec<String> {
    if cause == Cause::Confirmation {
        return Vec::new();
    }
    let was = prev.map_or(STATE_NONE, |p| p.state);
    let mut out = Vec::new();
    if now.state == STATE_NONE {
        if was != STATE_NONE {
            out.push("付款交易已被区块回滚撤销，订单暂时失效；如重新上链会再次通知".to_owned());
        }
        return out;
    }
    let fresh = was == STATE_NONE;
    let blank = ChatBrief::default();
    let p = if fresh {
        &blank
    } else {
        prev.unwrap_or(&blank)
    };
    if fresh {
        out.push(if now.instant {
            "买家已付款，资金已托管；即时发货，确认期开始".to_owned()
        } else {
            "买家已付款，资金已托管，等待卖家发货".to_owned()
        });
    } else if cause == Cause::Reorg {
        out.push("链上数据回滚，订单状态已按主链修正".to_owned());
    }
    if !now.instant {
        match (p.confirm_deadline, now.confirm_deadline) {
            (None, Some(_)) => out.push("卖家已标记发货，确认期开始".to_owned()),
            (Some(a), Some(b)) if b > a => out.push("确认期已延长".to_owned()),
            _ => {}
        }
    }
    if p.started_at.is_none() && now.started_at.is_some() {
        out.push("卖家已开始服务，按周期解锁".to_owned());
    }
    if let Some(by) = &now.proposer
        && (p.proposer.as_ref() != Some(by) || p.proposal_to_buyer != now.proposal_to_buyer)
    {
        out.push(format!(
            "{}提出和解提议，请在订单页查看并确认",
            side(by, &now.buyer)
        ));
    }
    if let Some(by) = &now.disputed_by
        && p.disputed_by.is_none()
    {
        out.push(format!("{}发起争议，进入举证期", side(by, &now.buyer)));
    }
    if now.state != p.state {
        match now.state {
            STATE_RELEASED => out.push("货款已放给卖家，订单完成".to_owned()),
            STATE_REFUNDED => out.push("已全额退款给买家，订单结束".to_owned()),
            STATE_SETTLED => out.push("订单已按和解或裁决结算".to_owned()),
            _ => {}
        }
    }
    out
}

/// 写订单聊天系统消息；订单刚付款时关联建单前的私信谈判记录。
pub async fn record_order(
    tx: &mut Transaction<'_, Postgres>,
    order_id: i64,
    prev: Option<&ChatBrief>,
    now: &ChatBrief,
    cause: Cause,
) -> Result<(), sqlx::Error> {
    for body in system_messages(prev, now, cause) {
        let m = order_chat::append(
            tx,
            NewOrderMessage {
                order_id,
                sender_id: None,
                kind: order_chat::KIND_SYSTEM,
                body: &body,
                warn: false,
            },
        )
        .await?;
        outbox::enqueue(
            &mut **tx,
            PUSH_ORDER_MESSAGE,
            &json!({ "id": m.id }),
            outbox::PRIORITY_NORMAL,
        )
        .await?;
    }
    if now.state != STATE_NONE && prev.is_none_or(|p| p.state == STATE_NONE) {
        order_chat::link_negotiation(&mut **tx, order_id).await?;
    }
    Ok(())
}

/// 私信报价卡片随链上成交变为「已付款」或回滚后恢复：在会话里写一张卡片消息并通知卖家。
pub async fn record_offer_fills(
    tx: &mut Transaction<'_, Postgres>,
    fills: &[OfferFill],
) -> Result<(), sqlx::Error> {
    for f in fills {
        let Some(conversation_id) = f.conversation_id else {
            continue;
        };
        let event = if f.status == OfferStatus::Paid {
            "paid"
        } else {
            "payment_reverted"
        };
        let m = messages::insert(
            &mut **tx,
            NewMessage {
                conversation_id,
                sender_id: None,
                kind: messages::KIND_OFFER,
                body: event,
                offer_id: Some(f.id),
                warn: false,
            },
        )
        .await?;
        outbox::enqueue(
            &mut **tx,
            PUSH_MESSAGE,
            &json!({ "id": m.id }),
            outbox::PRIORITY_NORMAL,
        )
        .await?;
        outbox::enqueue(
            &mut **tx,
            NOTIFY_OFFER,
            &json!({
                "to": f.seller_id, "from": f.buyer_id, "offer_id": f.id,
                "conversation_id": conversation_id, "event": event,
            }),
            outbox::PRIORITY_NORMAL,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const STATE_FUNDED: i16 = 1;
    const STATE_DISPUTED: i16 = 2;

    fn brief(state: i16) -> ChatBrief {
        ChatBrief {
            state,
            buyer: vec![0xb1; 20],
            ..ChatBrief::default()
        }
    }

    #[test]
    fn transitions() {
        let paid = brief(STATE_FUNDED);
        assert_eq!(
            system_messages(None, &paid, Cause::Event),
            ["买家已付款，资金已托管，等待卖家发货"]
        );
        // 只是确认级别变化
        assert!(system_messages(Some(&paid), &paid, Cause::Confirmation).is_empty());
        assert!(system_messages(Some(&paid), &paid, Cause::Event).is_empty());

        let t = time::OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
        let shipped = ChatBrief {
            confirm_deadline: Some(t),
            ..paid.clone()
        };
        assert_eq!(
            system_messages(Some(&paid), &shipped, Cause::Event),
            ["卖家已标记发货，确认期开始"]
        );
        let extended = ChatBrief {
            confirm_deadline: Some(t + time::Duration::days(1)),
            ..shipped.clone()
        };
        assert_eq!(
            system_messages(Some(&shipped), &extended, Cause::Event),
            ["确认期已延长"]
        );

        let disputed = ChatBrief {
            state: STATE_DISPUTED,
            disputed_by: Some(vec![0xc1; 20]),
            ..shipped.clone()
        };
        assert_eq!(
            system_messages(Some(&shipped), &disputed, Cause::Event),
            ["卖家发起争议，进入举证期"]
        );
        let proposal = ChatBrief {
            proposer: Some(vec![0xb1; 20]),
            proposal_to_buyer: Some("1".into()),
            ..shipped.clone()
        };
        assert_eq!(
            system_messages(Some(&shipped), &proposal, Cause::Event),
            ["买家提出和解提议，请在订单页查看并确认"]
        );
        let released = ChatBrief {
            state: STATE_RELEASED,
            ..shipped.clone()
        };
        assert_eq!(
            system_messages(Some(&shipped), &released, Cause::Event),
            ["货款已放给卖家，订单完成"]
        );

        // 即时发货：建单即有确认期限，不另写「已发货」
        let instant = ChatBrief {
            instant: true,
            confirm_deadline: Some(t),
            ..paid.clone()
        };
        assert_eq!(system_messages(None, &instant, Cause::Event).len(), 1);

        // 回滚：撤销与重放
        let none = brief(STATE_NONE);
        assert_eq!(system_messages(Some(&paid), &none, Cause::Reorg).len(), 1);
        assert_eq!(
            system_messages(Some(&none), &paid, Cause::Event),
            ["买家已付款，资金已托管，等待卖家发货"]
        );
        assert_eq!(
            system_messages(Some(&shipped), &paid, Cause::Reorg),
            ["链上数据回滚，订单状态已按主链修正"]
        );
    }
}
