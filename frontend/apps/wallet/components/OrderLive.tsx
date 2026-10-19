"use client";

import { ApiError } from "@akari/api-client";
import {
  Amount,
  Badge,
  Button,
  Card,
  ErrorState,
  LoadingState,
  OnceProgress,
  shortAddress,
  type OnceState,
} from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LoaderCircle } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { getAddress } from "viem";
import { useNow } from "../lib/useNow";
import { userMessage, walletApi, type OrderView } from "../lib/wallet/api";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import {
  availableAction,
  isFinal,
  isOnce,
  levelBadge,
  onceState,
  orderVersion,
  remaining,
  stateText,
  streamAction,
  type OrderAction,
} from "../lib/wallet/order";
import type { Wallet } from "../lib/wallet/sdk";
import { OrderList } from "./OrderList";
import { StreamPanel, useStreamOrder } from "./StreamOrder";
import { WalletGate } from "./WalletGate";

/** 订单状态每 3 秒重新拉一次（不用 SSE / WebSocket）；结局已定且最终确认后停止。 */
const POLL_MS = 3000;
/** 刚付款时 indexer 可能还没看到建单事件：这段时间内「找不到」按同步中处理。 */
const SYNC_GRACE_MS = 60_000;

/** 找不到订单返回 null（可能是 indexer 还没同步到），其他错误照常抛出。 */
async function fetchOrder(id: number): Promise<OrderView | null> {
  try {
    return await walletApi().order(id);
  } catch (e) {
    if (e instanceof ApiError && e.code === "NOT_FOUND") return null;
    throw e;
  }
}

function Deadline({
  order,
  state,
  now,
}: {
  order: OrderView;
  state: OnceState | null;
  now: number;
}) {
  const { ship, confirm, start } = order.deadlines;
  if (!isOnce(order)) {
    const endsAt = order.stream?.ends_at;
    if (state === "funded" && endsAt) return <>全部期数{remaining(endsAt, now)}到期。</>;
    return state === "funded" && start && !order.stream?.started_at ? (
      <>开始期限{remaining(start, now)}，逾期未开始买家可全额取回。</>
    ) : null;
  }
  if (state === "funded" && ship) {
    return <>发货期限{remaining(ship, now)}，逾期未发货买家可全额取回。</>;
  }
  if (state === "delivered" && confirm) {
    return <>确认期{remaining(confirm, now)}，期满未发起争议自动放款给卖家。</>;
  }
  return null;
}

/** 确认页上各操作的说明：动钱前先看清楚（plan 第 14 节）。 */
function confirmCopy(action: OrderAction, order: OrderView) {
  const free = { label: "代付费", value: "免费（结算时扣固定 gas 费）" };
  switch (action.kind) {
    case "release":
      return {
        amount: order.amount,
        rows: [free],
        note: `确认后货款立即放给卖家 ${shortAddress(order.seller.address)}，不能撤回。`,
      };
    case "markDelivered":
      return {
        amount: order.amount,
        rows: [free],
        note: "确认期从现在开始计算，期满未争议自动放款。",
      };
    case "start":
      return {
        amount: order.amount,
        rows: [free],
        note: "开始后按期解锁给你，任何一方都不能单方面停止；有问题请协商或发起争议。",
      };
    case "claim": {
      const payout = action.claim.ok ? action.claim.payout : 0n;
      return {
        amount: payout.toString(),
        rows: [
          { label: "代付费", value: "免费" },
          {
            label: "领取 gas 费",
            value:
              action.claim.ok && action.claim.settles ? "不收（结清订单）" : "已从领取额中扣除",
          },
        ],
        note: "领取的 USDT 直接转到你的钱包；两次领取至少间隔 24 小时。",
      };
    }
  }
}

async function submit(wallet: Wallet, action: OrderAction, orderId: bigint) {
  switch (action.kind) {
    case "markDelivered":
      return wallet.markDelivered(orderId);
    case "release":
      return wallet.release(orderId);
    case "start":
      return wallet.start(orderId);
    case "claim":
      return wallet.claim(orderId);
  }
}

/**
 * 订单页：状态来自 indexer 并自动刷新。一次性订单卖家标记发货、买家确认收货；分期订单卖家开始服务、
 * 领取已解锁部分，买家只看进度。动钱都先过确认页再 Face ID。
 */
export function OrderLive({ orderId, wallet }: { orderId: number; wallet: Wallet }) {
  const queryClient = useQueryClient();
  const [mountedAt] = useState(() => Date.now());
  const now = useNow();
  const [open, setOpen] = useState(false);
  /** 已提交上链、但 indexer 还没反映出来时的订单版本 */
  const [submittedAt, setSubmittedAt] = useState<string | null>(null);
  const query = useQuery({
    queryKey: ["order", orderId],
    queryFn: () => fetchOrder(orderId),
    refetchInterval: (q) => (q.state.data && isFinal(q.state.data) ? false : POLL_MS),
    refetchIntervalInBackground: false,
  });
  const stream = useStreamOrder(query.data);

  if (query.isPending) return <LoadingState rows={2} label="正在读取订单" />;
  if (query.isError && !query.data) {
    return <ErrorState message={userMessage(query.error, "订单读取失败，请稍后刷新")} />;
  }
  const order = query.data;
  if (!order) {
    return now - mountedAt < SYNC_GRACE_MS ? (
      <LoadingState rows={2} label="付款已上链，正在同步订单" />
    ) : (
      <ErrorState title="没有这个订单" message={`没有找到订单 #${orderId}`} />
    );
  }
  const text = stateText(order);
  if (!text) {
    return <ErrorState title="订单已撤销" message="这笔付款所在的区块被回滚，订单不存在了" />;
  }
  const state = onceState(order);
  const once = isOnce(order);

  const pending = submittedAt !== null && submittedAt === orderVersion(order);
  const action = pending
    ? null
    : once
      ? availableAction(order, wallet.address, now, chain.contracts.Escrow)
      : streamAction(order, stream.data, wallet.address, now, chain.contracts.StreamEscrow);
  const enabled = action !== null && (action.kind !== "claim" || action.claim.ok);
  const copy = action && enabled ? confirmCopy(action, order) : null;
  const me = wallet.address.toLowerCase();
  const buyer = order.buyer.address;
  const seller = order.seller.address;
  const role = buyer.toLowerCase() === me ? "买家" : seller.toLowerCase() === me ? "卖家" : null;
  const level = levelBadge(order);
  return (
    <div className="space-y-4">
      <div>
        <p className="text-sm text-fg-muted">
          订单 #{order.id} · {once ? "一次性担保" : "分期担保"}
          {role ? ` · 你是${role}` : ""}
        </p>
        <p className="mt-1 text-2xl font-semibold">
          <Amount value={order.amount} />
        </p>
      </div>
      <Card className="space-y-3">
        {once && state ? (
          <OnceProgress state={state} delivered={Boolean(order.deadlines.confirm)} />
        ) : null}
        {stream.data ? (
          <StreamPanel
            chainOrder={stream.data}
            action={role === "卖家" ? action : null}
            now={now}
          />
        ) : stream.isError ? (
          <p role="alert" className="text-sm text-warning">
            {userMessage(stream.error, "分期进度读取失败，请稍后刷新")}
          </p>
        ) : null}
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm font-medium" data-testid="order-state">
            {text}
          </p>
          <Badge tone={level.tone} data-testid="order-level">
            {level.label}
          </Badge>
        </div>
        <p className="text-sm text-fg-muted">
          <Deadline order={order} state={state} now={now} />
        </p>
        {query.isError ? (
          <p role="alert" className="text-sm text-warning">
            {userMessage(query.error, "网络异常")}，显示的是上次读取的状态
          </p>
        ) : null}
      </Card>
      <Card className="space-y-2 text-sm">
        <p>
          <span className="text-fg-muted">买家 </span>
          <span className="amount font-mono">{shortAddress(buyer)}</span>
        </p>
        <p>
          <span className="text-fg-muted">卖家 </span>
          <span className="amount font-mono">{shortAddress(seller)}</span>
        </p>
        {pending ? (
          <p className="flex items-center gap-2 text-fg-muted" role="status">
            <LoaderCircle size={16} className="animate-spin" aria-hidden />
            已提交上链，等待同步…
          </p>
        ) : null}
        {action ? (
          <Button className="mt-2" disabled={!enabled} onClick={() => setOpen(true)}>
            {action.label}
          </Button>
        ) : null}
      </Card>
      {action && copy ? (
        <SignConfirmDialog
          open={open}
          onOpenChange={setOpen}
          action={action.label}
          amount={copy.amount}
          to={getAddress(order.contract)}
          toLabel={once ? "担保合约" : "分期担保合约"}
          network={NETWORK_LABEL}
          rows={copy.rows}
          note={copy.note}
          onConfirm={async () => {
            await submit(wallet, action, BigInt(order.chain_order_id));
            setOpen(false);
            setSubmittedAt(orderVersion(order));
            await queryClient.invalidateQueries({ queryKey: ["order", orderId] });
          }}
        />
      ) : null}
      {role === "买家" && state === "delivered" && once ? (
        <p className="text-sm text-fg-muted">有问题请在确认期满前发起争议，否则自动放款给卖家。</p>
      ) : null}
    </div>
  );
}

const ORDER_ID = /^[1-9]\d{0,15}$/;

/** `/order?id=<订单 ID>`：静态导出没有动态路由参数，用查询参数；不带 id 时显示我的订单列表。 */
export function OrderSection() {
  const id = useSearchParams().get("id");
  if (id !== null && !ORDER_ID.test(id)) {
    return <ErrorState title="订单号无效" message="请从订单列表或付款结果页进入" />;
  }
  return (
    <WalletGate>
      {(wallet) => (id ? <OrderLive orderId={Number(id)} wallet={wallet} /> : <OrderList />)}
    </WalletGate>
  );
}
