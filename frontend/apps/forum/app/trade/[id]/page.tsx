import { Amount, Badge, Button, Card, NetworkBadge, UserCard } from "@akari/ui";
import { orderCostPreview } from "../../../lib/fees";
import { Clock, ShieldCheck, Zap } from "lucide-react";
import { SiteNav } from "../../../components/SiteNav";
import { Prose } from "../../../components/Prose";
import { trade, user } from "../../../lib/mock";

const WALLET_ORIGIN = process.env["NEXT_PUBLIC_WALLET_ORIGIN"] ?? "http://localhost:3001";

export default function TradePage() {
  const s = user(trade.seller);
  const cost = orderCostPreview(trade.price, trade.feePayer);
  return (
    <>
      <SiteNav active="/f/digital" />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-[1fr_20rem]">
        <section className="min-w-0 space-y-3">
          <nav aria-label="位置" className="text-sm text-fg-muted">
            <a href="/" className="hover:text-fg">
              首页
            </a>{" "}
            /{" "}
            <a href="/f/digital" className="hover:text-fg">
              数字商品交易
            </a>
          </nav>
          <h1 className="text-2xl leading-snug font-semibold">
            <Badge tone="warning" className="mr-2 align-[4px]">
              担保
            </Badge>
            {trade.title}
          </h1>
          <Card>
            <UserCard
              name={s.name}
              level={s.level}
              levelName={s.levelName}
              online={s.online}
              reputation={s.reputation}
              href={`/u/${encodeURIComponent(s.name)}`}
            />
            <div className="mt-4">
              <Prose html={trade.html} />
            </div>
            <p className="mt-4 text-xs text-fg-muted">
              内容快照 {trade.snapshotHash} · 成交后以下单时的版本为准，卖家之后编辑不影响已成交订单
            </p>
          </Card>
        </section>
        <aside className="min-w-0">
          <Card className="space-y-4 lg:sticky lg:top-20">
            <div>
              <p className="text-sm text-fg-muted">价格</p>
              <p className="text-3xl font-semibold">
                <Amount value={trade.price} />
              </p>
            </div>
            <ul className="space-y-2 text-sm">
              <li className="flex items-center gap-2">
                <Zap size={16} className="text-accent" aria-hidden />
                即时发货：付款确认后自动发卡密
              </li>
              <li className="flex items-center gap-2">
                <Clock size={16} className="text-accent" aria-hidden />
                买家确认期 {trade.confirmDays} 天，期满自动放款
              </li>
              <li className="flex items-center gap-2">
                <ShieldCheck size={16} className="text-accent" aria-hidden />
                卖家保证金 <Amount value={trade.bond} />
                ，将随订单锁定
              </li>
            </ul>
            <dl className="space-y-1 rounded-lg bg-surface-2 p-3 text-sm">
              <div className="flex justify-between">
                <dt className="text-fg-muted">卖家承担的手续费与 gas 费</dt>
                <dd>
                  <Amount value={cost.sellerFee} />
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-fg-muted">你需支付（含 0.05 代付费）</dt>
                <dd className="font-medium">
                  <Amount value={cost.buyerPays} />
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-fg-muted">本单争议费（发起方预付）</dt>
                <dd>
                  <Amount value={cost.disputeFee} />
                </dd>
              </div>
            </dl>
            <div className="flex items-center justify-between text-sm">
              <NetworkBadge />
              <span className="text-fg-muted">库存 {trade.stock}</span>
            </div>
            <Button asChild size="lg" className="w-full">
              <a href={`${WALLET_ORIGIN}/checkout?listing=${trade.id}`}>担保购买</a>
            </Button>
            <Button variant="secondary" className="w-full">
              私信卖家议价
            </Button>
          </Card>
        </aside>
      </main>
    </>
  );
}
