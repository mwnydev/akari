import { Amount, Badge, Card, OnceProgress, StreamProgress, UserCard } from "@akari/ui";
import { ChatBox } from "@akari/ui/chat";
import { KeyRound } from "lucide-react";
import { notFound } from "next/navigation";
import { OrderActions } from "../../../components/OrderActions";
import { WalletNav } from "../../../components/WalletNav";
import { chat, orderIds, orders, type OrderId } from "../../../lib/mock";

export function generateStaticParams() {
  return orderIds.map((id) => ({ id }));
}

export default async function OrderPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  if (!(id in orders)) notFound();
  const o = orders[id as OrderId];
  return (
    <>
      <WalletNav />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-[1fr_24rem]">
        <section className="min-w-0 space-y-4">
          <div>
            <p className="text-sm text-fg-muted">
              订单 #{o.id} · {o.mode === "once" ? "一次性担保" : "分期担保"}
            </p>
            <h1 className="text-xl font-semibold">{o.title}</h1>
            <p className="mt-1 text-2xl font-semibold">
              <Amount value={o.amount} />
            </p>
          </div>
          <Card>
            {o.mode === "once" ? (
              <OnceProgress state={o.state} hint={o.hint} />
            ) : (
              <StreamProgress
                periodLabel="每天解锁"
                periods={o.periods}
                elapsed={o.elapsed}
                net={o.net}
                unlocked={o.unlocked}
                claimed={o.claimed}
              />
            )}
          </Card>
          {o.mode === "once" ? (
            <Card className="flex items-center gap-3">
              <KeyRound className="shrink-0 text-accent" aria-hidden />
              <div className="min-w-0">
                <p className="text-sm text-fg-muted">卡密（已上链确认后自动发放）</p>
                <p className="amount font-mono text-lg break-all">{o.card}</p>
              </div>
              <Badge tone="success" className="ml-auto shrink-0">
                已发货
              </Badge>
            </Card>
          ) : null}
          <Card>
            <OrderActions mode={o.mode} id={o.id} />
            <p className="mt-3 text-sm text-fg-muted">
              无论对方失联还是平台停摆，资金都能经超时规则取回：卖家逾期不发货全额退款；争议 30
              天未裁决按默认规则结算。
            </p>
          </Card>
        </section>
        <ChatBox
          header={
            <UserCard
              name={o.counterpart.name}
              level={o.counterpart.level}
              online={o.counterpart.online}
              reputation={o.counterpart.reputation}
              size="sm"
            />
          }
          messages={chat}
          typing
        />
      </main>
    </>
  );
}
