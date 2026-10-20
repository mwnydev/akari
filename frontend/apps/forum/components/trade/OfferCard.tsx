import type { Schemas } from "@akari/api-client";
import { Amount, Badge, Button, Card, type BadgeTone } from "@akari/ui";
import type { ReactNode } from "react";
import { formatDays, formatTime } from "../../lib/display";
import { FEE_PAYERS } from "../../lib/fees";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { WalletLink } from "../settings/UsernameGate";

const ZERO_ADDRESS = /^0x0{40}$/i;

const STATUS: Record<Schemas["CardStatus"], [string, BadgeTone]> = {
  pending: ["可购买", "success"],
  modified: ["已被新报价替换", "neutral"],
  rejected: ["已拒绝", "neutral"],
  cancelled: ["卖家已取消", "neutral"],
  expired: ["已过期", "neutral"],
  paid: ["已成交", "accent"],
};

/**
 * 卖家签名的报价卡片：条款以签名内容为准；待确认的报价可以去钱包担保购买。
 * `canPay` 为假时不显示购买按钮（卖家本人看自己发出的卡片）；`children` 放在卡片底部（拒绝、取消、查看订单）。
 */
export function OfferCard({
  offer,
  canPay = true,
  children,
}: {
  offer: Schemas["OfferView"];
  canPay?: boolean;
  children?: ReactNode;
}) {
  const o = offer.offer;
  const [statusLabel, tone] = STATUS[offer.status];
  const isPublic = ZERO_ADDRESS.test(o.buyer);
  return (
    <Card className="space-y-3" data-testid="offer-card">
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm text-fg-muted">{isPublic ? "公开报价" : "指定买家报价"}</span>
        <Badge tone={tone}>{statusLabel}</Badge>
      </div>
      <p className="text-2xl font-semibold">
        <Amount value={o.amount} />
      </p>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-sm">
        <dt className="text-fg-muted">发货</dt>
        <dd>{o.instant ? "付款后即时发货" : `${formatDays(o.shipWindow)}内`}</dd>
        <dt className="text-fg-muted">确认期</dt>
        <dd>{formatDays(o.confirmWindow)}</dd>
        <dt className="text-fg-muted">手续费</dt>
        <dd>{FEE_PAYERS[o.feePayer]?.label ?? "—"}</dd>
        {o.bond !== "0" ? (
          <>
            <dt className="text-fg-muted">卖家保证金</dt>
            <dd>
              <Amount value={o.bond} />
            </dd>
          </>
        ) : null}
        <dt className="text-fg-muted">有效期至</dt>
        <dd>
          <time dateTime={offer.expires_at}>{formatTime(offer.expires_at)}</time>
        </dd>
      </dl>
      {offer.status === "pending" && canPay ? (
        <Button asChild size="lg" className="w-full">
          <WalletLink href={`${WALLET_ORIGIN}/pay?offer=${offer.id}`}>担保购买</WalletLink>
        </Button>
      ) : null}
      {children}
    </Card>
  );
}
