import { ApiError, type Schemas } from "@akari/api-client";
import { Amount, Badge, Button, Card, NetworkBadge, UserCard, type BadgeTone } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";
import { OrderCasePanel } from "../../../components/arbitration/OrderCasePanel";
import { PresenceDot } from "../../../components/live/PresenceDot";
import { OrderChat } from "../../../components/orders/OrderChat";
import { PageError } from "../../../components/PageError";
import { AppShell } from "../../../components/forum/AppShell";
import { load, serverApi } from "../../../lib/api";
import {
  cardLookup,
  formatTime,
  profileHref,
  tradeReputation,
  userName,
} from "../../../lib/display";
import { topicHref } from "../../../lib/links";
import { ORDER_LEVEL, ORDER_STATE } from "../../../lib/orders";
import { WALLET_ORIGIN } from "../../../lib/public-config";
import { requireMe } from "../../../lib/session";

export const metadata: Metadata = { title: "订单", robots: { index: false } };

const STATE_TONE: Record<string, BadgeTone> = {
  funded: "accent",
  disputed: "danger",
  released: "success",
  refunded: "neutral",
  settled: "neutral",
  none: "warning",
};

function Party({
  role,
  party,
  card,
}: {
  role: string;
  party: Schemas["Party"];
  card: Schemas["UserCardView"] | null;
}) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-fg-muted">{role}</p>
      {card ? (
        <UserCard
          name={userName(card)}
          avatar={card.avatar_url ?? undefined}
          level={card.level}
          reputation={tradeReputation(card.reputation)}
          href={profileHref(card)}
          avatarBadge={<PresenceDot userId={card.id} avatarSize={32} />}
          size="sm"
        />
      ) : (
        <p className="amount text-sm break-all">{party.address}</p>
      )}
    </div>
  );
}

/**
 * 订单的论坛侧页面：摘要、双方名片与订单聊天（带哈希链校验）。发货、确认、争议等要签名的操作在钱包里做
 * （独立域名，plan 第 5 节）。只有买卖双方与仲裁员能打开，其他人 404。
 */
export default async function OrderPage({ params }: { params: Promise<{ id: string }> }) {
  const raw = (await params).id;
  if (!/^\d{1,15}$/.test(raw)) notFound();
  const id = Number(raw);
  const me = await requireMe(`/orders/${id}`);
  const api = await serverApi();
  const order = await load(api.GET("/api/v1/orders/{id}", { params: { path: { id } } }));
  if (order instanceof ApiError) {
    if (order.status === 404) notFound();
    return (
      <AppShell>
        <PageError error={order} />
      </AppShell>
    );
  }
  const ids = [order.buyer.user_id, order.seller.user_id].filter((u): u is number => u != null);
  const cards = ids.length
    ? await load(api.GET("/api/v1/users/cards", { params: { query: { ids: ids.join(",") } } }))
    : [];
  const card = cardLookup(cards instanceof ApiError ? [] : cards);
  const cardOf = (p: Schemas["Party"]) => (p.user_id != null ? card(p.user_id) : null);
  const names: Record<number, string> = {};
  for (const p of [order.buyer, order.seller]) {
    if (p.user_id != null) names[p.user_id] = userName(card(p.user_id));
  }
  const d = order.deadlines;

  return (
    <AppShell>
      <div className="grid max-w-6xl gap-4 xl:grid-cols-[minmax(0,1fr)_24rem]">
        <section className="min-w-0 space-y-4">
          <div>
            <p className="text-sm text-fg-muted">
              订单 #{order.id} · {order.mode === "stream" ? "分期担保" : "一次性担保"}
            </p>
            <h1 className="mt-1 flex flex-wrap items-center gap-2 text-2xl font-semibold">
              <Amount value={order.amount} />
              <Badge tone={STATE_TONE[order.state] ?? "neutral"} data-testid="order-state">
                {ORDER_STATE[order.state] ?? order.state}
              </Badge>
              <Badge>{ORDER_LEVEL[order.level] ?? order.level}</Badge>
            </h1>
          </div>
          <Card className="grid gap-4 sm:grid-cols-2">
            <Party role="买家" party={order.buyer} card={cardOf(order.buyer)} />
            <Party role="卖家" party={order.seller} card={cardOf(order.seller)} />
          </Card>
          <Card className="space-y-3 text-sm">
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
              {d.ship ? (
                <>
                  <dt className="text-fg-muted">发货期限</dt>
                  <dd>{formatTime(d.ship)}</dd>
                </>
              ) : null}
              {d.confirm ? (
                <>
                  <dt className="text-fg-muted">确认期限</dt>
                  <dd>{formatTime(d.confirm)}</dd>
                </>
              ) : null}
              {d.start ? (
                <>
                  <dt className="text-fg-muted">开始服务期限</dt>
                  <dd>{formatTime(d.start)}</dd>
                </>
              ) : null}
              <dt className="text-fg-muted">下单时间</dt>
              <dd>{formatTime(order.created_at)}</dd>
            </dl>
            <NetworkBadge />
            <div className="flex flex-wrap gap-2">
              <Button asChild>
                <a href={`${WALLET_ORIGIN}/order?id=${order.id}`}>去钱包处理订单</a>
              </Button>
              {order.source.listing_topic_id != null ? (
                <Button asChild variant="secondary">
                  <Link href={topicHref(order.source.listing_topic_id)}>查看交易帖</Link>
                </Button>
              ) : null}
              {order.negotiation && ids.includes(me.id) ? (
                <Button asChild variant="secondary">
                  <Link href={`/messages/${order.negotiation.conversation_id}`}>建单前的私信</Link>
                </Button>
              ) : null}
            </div>
            <p className="text-fg-muted">
              发货、确认收货、和解与争议都要在钱包里用 Passkey
              签名；这里的聊天记录只追加、带哈希链， 可作为仲裁证据。
            </p>
          </Card>
          {order.dispute || order.state === "disputed" ? (
            <OrderCasePanel orderId={order.id} />
          ) : null}
        </section>
        <OrderChat orderId={order.id} me={me.id} names={names} />
      </div>
    </AppShell>
  );
}
