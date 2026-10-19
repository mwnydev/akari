"use client";

import {
  Amount,
  Button,
  Card,
  ErrorState,
  LoadingState,
  OnceProgress,
  shortAddress,
  type OnceState,
} from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import type { OrderView, Wallet } from "../lib/wallet/sdk";
import { WalletGate } from "./WalletGate";

/** Escrow.State：None, Funded, Disputed, Released, Refunded, Settled */
function onceState(o: OrderView): OnceState | null {
  switch (o.state) {
    case 1:
      return o.confirmDeadline === 0n ? "funded" : "delivered";
    case 2:
      return "disputed";
    case 3:
      return "released";
    case 4:
      return "refunded";
    case 5:
      return "settled";
    default:
      return null;
  }
}

type Action = { kind: "markDelivered"; label: string } | { kind: "release"; label: string };

function availableAction(o: OrderView, me: string): Action | null {
  if (o.state !== 1) return null;
  const mine = (a: string) => a.toLowerCase() === me.toLowerCase();
  if (mine(o.seller) && o.confirmDeadline === 0n)
    return { kind: "markDelivered", label: "标记已发货" };
  if (mine(o.buyer)) return { kind: "release", label: "确认收货" };
  return null;
}

/** 订单页（链上实时状态）：卖家标记发货、买家确认收货，都先过确认页再 Face ID；完成后自动刷新。 */
export function OrderLive({ orderId, wallet }: { orderId: bigint; wallet: Wallet }) {
  const [order, setOrder] = useState<OrderView | null>(null);
  const [error, setError] = useState("");
  const [open, setOpen] = useState(false);

  const refresh = useCallback(async () => {
    const sdk = await loadWalletSdk();
    setOrder(await sdk.readOrder(orderId));
  }, [orderId]);

  useEffect(() => {
    refresh().catch(() => setError("订单读取失败，请稍后刷新"));
  }, [refresh]);

  if (error) return <ErrorState message={error} />;
  if (!order) return <LoadingState rows={2} label="正在读取订单" />;
  const state = onceState(order);
  if (!state) return <ErrorState title="没有这个订单" message={`订单 #${orderId} 不存在`} />;

  const action = availableAction(order, wallet.address);
  const iAmBuyer = order.buyer.toLowerCase() === wallet.address.toLowerCase();
  return (
    <div className="space-y-4">
      <div>
        <p className="text-sm text-fg-muted">订单 #{orderId.toString()} · 一次性担保</p>
        <p className="mt-1 text-2xl font-semibold">
          <Amount value={order.amount} />
        </p>
      </div>
      <Card>
        <OnceProgress state={state} delivered={order.confirmDeadline !== 0n} />
        <p className="mt-3 text-sm text-fg-muted" data-testid="order-state">
          {
            {
              funded: "买家已付款，等待卖家发货",
              delivered: "卖家已发货，等待买家确认收货",
              released: "已确认收货，货款已放给卖家",
              refunded: "已退款给买家",
              settled: "已协商结算",
              disputed: "争议中",
            }[state]
          }
        </p>
      </Card>
      <Card className="space-y-2 text-sm">
        <p>
          <span className="text-fg-muted">买家 </span>
          <span className="amount font-mono">{shortAddress(order.buyer)}</span>
        </p>
        <p>
          <span className="text-fg-muted">卖家 </span>
          <span className="amount font-mono">{shortAddress(order.seller)}</span>
        </p>
        {action ? (
          <Button className="mt-2" onClick={() => setOpen(true)}>
            {action.label}
          </Button>
        ) : null}
      </Card>
      {action ? (
        <SignConfirmDialog
          open={open}
          onOpenChange={setOpen}
          action={action.label}
          amount={order.amount.toString()}
          to={chain.contracts.Escrow}
          toLabel="担保合约"
          network={NETWORK_LABEL}
          rows={[{ label: "代付费", value: "免费（结算时扣固定 gas 费）" }]}
          note={
            action.kind === "release"
              ? `确认后货款立即放给卖家 ${shortAddress(order.seller)}，不能撤回。`
              : "确认期从现在开始计算，期满未争议自动放款。"
          }
          onConfirm={async () => {
            await (action.kind === "release"
              ? wallet.release(orderId)
              : wallet.markDelivered(orderId));
            setOpen(false);
            await refresh();
          }}
        />
      ) : null}
      {iAmBuyer && state === "delivered" ? (
        <p className="text-sm text-fg-muted">有问题请在确认期满前发起争议，否则自动放款给卖家。</p>
      ) : null}
    </div>
  );
}

/** `/order?id=…`：静态导出没有动态路由参数，链上订单用查询参数（设计稿里的 /orders/[id] 仍是模拟数据）。 */
export function OrderSection() {
  const id = useSearchParams().get("id");
  if (!id || !/^[1-9]\d{0,18}$/.test(id)) {
    return <ErrorState title="订单号无效" message="请从订单列表或付款结果页进入" />;
  }
  return <WalletGate>{(wallet) => <OrderLive orderId={BigInt(id)} wallet={wallet} />}</WalletGate>;
}
