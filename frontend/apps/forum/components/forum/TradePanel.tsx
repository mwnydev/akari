import { Amount, Badge, Card, NetworkBadge } from "@akari/ui";
import { Clock, Hand, Package, ShieldCheck, Zap } from "lucide-react";
import type { ReactNode } from "react";
import { formatDays } from "../../lib/display";
import { cnyReference, formatCnyPerUsdt } from "../../lib/fees";
import { EscrowSteps } from "./EscrowSteps";

export interface ListingTermsView {
  /** USDT 最小单位 */
  price: string;
  cnyPerUsdt?: number | null | undefined;
  instant: boolean;
  /** 人工发货的发货期限、收货确认期（秒） */
  shipWindow: number;
  confirmWindow: number;
  /** 卖家保证金，USDT 最小单位；"0" = 无 */
  bond: string;
  /** 库存；接口没给时不显示 */
  stock?: number | null | undefined;
  /** 手续费由谁承担（「卖家承担」等） */
  feeLabel: string;
  /** 买家应付（含代付费）；交易参数取不到时不显示 */
  buyerPays?: string | undefined;
}

/**
 * 交易帖的价格与条款面板：标价（USDT + 可选人民币参考）、发货方式、期限、保证金、应付金额、
 * 操作按钮（`actions`）与担保说明。
 */
export function TradePanel({ terms: t, actions }: { terms: ListingTermsView; actions: ReactNode }) {
  const confirmDays = formatDays(t.confirmWindow);
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
            {t.instant ? "付款后系统即时发卡" : `付款后 ${formatDays(t.shipWindow)}内发货`}
          </span>
        </dd>
        <dt className="flex items-center gap-1.5 text-fg-muted">
          <Clock size={15} aria-hidden />
          确认期
        </dt>
        <dd>收货后 {confirmDays}，期满自动放款</dd>
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
        {t.stock != null ? (
          <>
            <dt className="flex items-center gap-1.5 text-fg-muted">
              <Package size={15} aria-hidden />
              库存
            </dt>
            <dd className="amount">{t.stock} 件</dd>
          </>
        ) : null}
      </dl>
      {t.buyerPays ? (
        <div className="flex items-baseline justify-between gap-2 rounded-lg bg-surface-2 px-3 py-2 text-sm">
          <span className="text-fg-muted">你需支付（手续费{t.feeLabel}，含 0.05 代付费）</span>
          <Amount value={t.buyerPays} className="font-medium" />
        </div>
      ) : null}
      <div className="space-y-2">{actions}</div>
      <EscrowSteps confirmDays={confirmDays} />
      <NetworkBadge />
    </Card>
  );
}
