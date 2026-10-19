import { DEFAULT_PARAMS } from "@akari/fees";
import { Amount, Badge, Button, Card, NetworkBadge } from "@akari/ui";
import { Clock, Hand, Package, ShieldCheck, Zap } from "lucide-react";
import { cnyReference, formatCnyPerUsdt, orderCostPreview } from "../../../../lib/fees";
import { EscrowSteps } from "./EscrowSteps";

export interface ListingTerms {
  /** USDT 最小单位 */
  price: string;
  cnyPerUsdt?: number;
  instant: boolean;
  /** 人工发货的发货期限（天） */
  shipDays: number;
  confirmDays: number;
  /** 卖家保证金，USDT 最小单位；"0" = 无 */
  bond: string;
  stock: number;
}

/** 交易帖的价格与条款面板：标价（USDT + 可选人民币参考）、发货方式、期限、保证金、担保说明与操作。 */
export function TradePanel({ terms: t }: { terms: ListingTerms }) {
  const cost = orderCostPreview(t.price, "seller", {
    fee_bps: Number(DEFAULT_PARAMS.feeBps),
    min_fee: DEFAULT_PARAMS.minFee.toString(),
    base_gas: DEFAULT_PARAMS.baseGas.toString(),
  });
  return (
    <Card className="space-y-4">
      <div>
        <p className="text-sm text-fg-muted">标价</p>
        <p className="text-3xl font-semibold">
          <Amount value={t.price} />
        </p>
        {t.cnyPerUsdt ? (
          <p className="amount mt-0.5 text-sm text-fg-muted">
            ≈ {cnyReference(t.price, t.cnyPerUsdt)}（卖家参考汇率 {formatCnyPerUsdt(t.cnyPerUsdt)}）
          </p>
        ) : null}
        <p className="mt-1 text-xs text-fg-muted">以 USDT 付款；人民币仅供参考，价格可私信商量</p>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2 text-sm">
        <dt className="flex items-center gap-1.5 text-fg-muted">
          {t.instant ? <Zap size={15} aria-hidden /> : <Hand size={15} aria-hidden />}
          发货方式
        </dt>
        <dd>
          {t.instant ? <Badge tone="success">自动发卡</Badge> : <Badge>人工发货</Badge>}
          <span className="ml-1.5 text-fg-muted">
            {t.instant ? "付款后系统即时发卡" : `付款后 ${t.shipDays} 天内发货`}
          </span>
        </dd>
        <dt className="flex items-center gap-1.5 text-fg-muted">
          <Clock size={15} aria-hidden />
          确认期
        </dt>
        <dd>收货后 {t.confirmDays} 天，期满自动放款</dd>
        <dt className="flex items-center gap-1.5 text-fg-muted">
          <ShieldCheck size={15} aria-hidden />
          卖家保证金
        </dt>
        <dd>
          {t.bond === "0" ? (
            "无"
          ) : (
            <>
              <Amount value={t.bond} />
              <span className="block text-xs text-fg-muted">
                随订单锁定，超时未发货按比例赔给你
              </span>
            </>
          )}
        </dd>
        <dt className="flex items-center gap-1.5 text-fg-muted">
          <Package size={15} aria-hidden />
          库存
        </dt>
        <dd className="amount">{t.stock} 件</dd>
      </dl>
      <div className="flex items-baseline justify-between gap-2 rounded-lg bg-surface-2 px-3 py-2 text-sm">
        <span className="text-fg-muted">你需支付（手续费由卖家承担，含 0.05 代付费）</span>
        <Amount value={cost.buyerPays} className="font-medium" />
      </div>
      <div className="space-y-2">
        <Button size="lg" className="w-full">
          <ShieldCheck size={18} aria-hidden />
          担保购买
        </Button>
        <div className="grid grid-cols-2 gap-2">
          <Button variant="secondary">私信卖家</Button>
          <Button variant="secondary">发起担保</Button>
        </div>
        <p className="text-xs text-fg-muted">
          「发起担保」：在私信里提出自定义数量、价格或保证金，卖家同意后签名报价。
        </p>
      </div>
      <EscrowSteps confirmDays={t.confirmDays} />
      <NetworkBadge />
    </Card>
  );
}
