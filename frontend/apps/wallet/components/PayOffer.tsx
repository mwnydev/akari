"use client";

import { ApiError } from "@akari/api-client";
import {
  Amount,
  Button,
  Card,
  CardTitle,
  ErrorState,
  LoadingState,
  NetworkBadge,
  shortAddress,
} from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { useQuery } from "@tanstack/react-query";
import { CircleCheck, LoaderCircle, ScanFace } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { userMessage, walletApi } from "../lib/wallet/api";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import type { Checkout, Wallet } from "../lib/wallet/sdk";
import { WalletGate } from "./WalletGate";

const DAY = 86_400;
/** 付款上链后等 indexer 同步出订单（GET /offers/{id}/order）的轮询间隔。 */
const SYNC_POLL_MS = 2000;

/** 这张报价卡片生成的订单；indexer 还没同步到（或没付过款）时返回 null。 */
async function fetchOfferOrder(offerId: number) {
  try {
    return await walletApi().offerOrder(offerId);
  } catch (e) {
    if (e instanceof ApiError && e.code === "NOT_FOUND") return null;
    throw e;
  }
}

/** 订单页按站内订单 ID（`OrderView.id`）打开，不是链上订单号。 */
function OrderLink({ orderId, children }: { orderId: number; children: string }) {
  return (
    <Button asChild>
      <a data-testid="order-link" href={`/order?id=${orderId}`}>
        {children}
      </a>
    </Button>
  );
}

/** 卡片已失效时看看是不是已经付过款（买家刷新了付款页），是就直接带去订单。 */
function Unavailable({ offerId, message }: { offerId: number; message: string }) {
  const order = useQuery({
    queryKey: ["offer-order", offerId],
    queryFn: () => fetchOfferOrder(offerId),
    retry: false,
  });
  if (order.isPending) return <LoadingState rows={1} label="正在查询订单" />;
  if (order.data) {
    return (
      <Card className="space-y-3">
        <p className="font-medium">这份报价已经付款，订单 #{order.data.id}</p>
        <OrderLink orderId={order.data.id}>查看订单</OrderLink>
      </Card>
    );
  }
  return <ErrorState title="不能付款" message={message} />;
}

/** 付款已上链：等 indexer 同步出订单后给出订单链接。 */
function Paid({ offerId }: { offerId: number }) {
  const order = useQuery({
    queryKey: ["offer-order", offerId, "paid"],
    queryFn: () => fetchOfferOrder(offerId),
    refetchInterval: (q) => (q.state.data ? false : SYNC_POLL_MS),
  });
  return (
    <Card className="space-y-3">
      <p className="flex items-center gap-2 font-medium text-success">
        <CircleCheck size={20} aria-hidden />
        付款成功{order.data ? `，订单 #${order.data.id} 已建立` : ""}
      </p>
      {order.data ? (
        <OrderLink orderId={order.data.id}>查看订单</OrderLink>
      ) : order.isError ? (
        <p role="alert" className="text-sm text-warning">
          {userMessage(order.error, "订单读取失败")}，稍后可在「我的订单」里查看
        </p>
      ) : (
        <p className="flex items-center gap-2 text-sm text-fg-muted" role="status">
          <LoaderCircle size={16} className="animate-spin" aria-hidden />
          付款已上链，正在同步订单…
        </p>
      )}
    </Card>
  );
}

function Confirm({
  checkout,
  wallet,
  onPaid,
}: {
  checkout: Checkout;
  wallet: Wallet;
  onPaid: () => void;
}) {
  const [open, setOpen] = useState(false);
  const { signed, quote, balance } = checkout;
  const { offer } = signed;
  const enough = balance >= quote.total;
  return (
    <Card className="space-y-3">
      <CardTitle>确认下单</CardTitle>
      <dl className="space-y-2 text-sm">
        {[
          {
            label: "卖家",
            value: <span className="amount font-mono">{shortAddress(offer.seller)}</span>,
          },
          {
            label: "发货期 / 确认期",
            value: `${offer.shipWindow / DAY} 天 / ${offer.confirmWindow / DAY} 天`,
          },
          { label: "货款", value: <Amount value={offer.amount} /> },
          {
            label: "你承担的手续费与 gas 费",
            value: <Amount value={quote.buyerPays - offer.amount} />,
          },
          { label: "代付费", value: <Amount value={quote.sponsorFee} /> },
        ].map((r) => (
          <div key={r.label} className="flex justify-between gap-3">
            <dt className="text-fg-muted">{r.label}</dt>
            <dd>{r.value}</dd>
          </div>
        ))}
        <div className="flex justify-between gap-3 border-t border-border pt-2 text-lg font-semibold">
          <dt>合计</dt>
          <dd data-testid="pay-total">
            <Amount value={quote.total} />
          </dd>
        </div>
      </dl>
      <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
        <span className="text-fg-muted">
          钱包余额 <Amount value={balance} />
        </span>
        <NetworkBadge label={NETWORK_LABEL} />
      </div>
      {enough ? (
        <Button size="lg" className="w-full" onClick={() => setOpen(true)}>
          <ScanFace size={20} aria-hidden />
          付款 <Amount value={quote.total} />
        </Button>
      ) : (
        <div className="space-y-1 rounded-lg bg-warning-soft p-3 text-sm text-warning">
          <p>余额不足，请先从交易所选 Arbitrum One 网络提币到钱包：</p>
          <p className="amount font-mono break-all">{wallet.address}</p>
        </div>
      )}
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action="担保付款"
        amount={quote.total.toString()}
        to={chain.contracts.Escrow}
        toLabel="担保合约"
        network={NETWORK_LABEL}
        rows={[
          { label: "货款（托管至确认收货）", value: <Amount value={offer.amount} /> },
          { label: "代付费", value: <Amount value={quote.sponsorFee} /> },
        ]}
        note="授权与付款合成一次 Face ID。付款成功的那一刻订单才成立；余额不足时整笔不会上链，也不扣代付费。"
        onConfirm={async () => {
          await wallet.pay(signed, quote);
          setOpen(false);
          onPaid();
        }}
      />
    </Card>
  );
}

/** 买家：后端给出付款数据 → 核对网络、钱包、卖家签名与费用 → 确认页 → 一次 Face ID 完成授权 + 付款建单。 */
export function PayOffer({ offerId, wallet }: { offerId: number; wallet: Wallet }) {
  const [paid, setPaid] = useState(false);
  const checkout = useQuery({
    queryKey: ["checkout", offerId, wallet.address],
    queryFn: async () => (await loadWalletSdk()).prepareCheckout(walletApi(), wallet, offerId),
    retry: false,
    staleTime: Infinity,
    structuralSharing: false,
  });

  if (paid) return <Paid offerId={offerId} />;
  if (checkout.isPending) return <LoadingState rows={2} label="正在核对报价" />;
  if (checkout.isError) {
    const message = userMessage(checkout.error, "报价校验失败，请稍后再试");
    return checkout.error instanceof ApiError && checkout.error.code === "OFFER_UNAVAILABLE" ? (
      <Unavailable offerId={offerId} message={message} />
    ) : (
      <ErrorState title="不能付款" message={message} />
    );
  }
  return <Confirm checkout={checkout.data} wallet={wallet} onPaid={() => setPaid(true)} />;
}

/** `/pay?offer=<报价卡片 ID>`（页面需包在 Suspense 里，静态导出读查询参数的要求）。 */
export function PaySection() {
  const raw = useSearchParams().get("offer");
  if (!raw || !/^[1-9]\d{0,15}$/.test(raw)) {
    return <ErrorState title="付款链接无效" message="链接不完整，请从私信里的报价卡片进入" />;
  }
  const offerId = Number(raw);
  return <WalletGate>{(wallet) => <PayOffer offerId={offerId} wallet={wallet} />}</WalletGate>;
}
