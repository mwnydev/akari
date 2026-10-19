"use client";

import { Amount, StreamProgress } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import type { OrderView } from "../lib/wallet/api";
import { chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import { isOnce, orderVersion, remaining, type OrderAction } from "../lib/wallet/order";
import { StreamState, periodLabel, streamProgress, type StreamOrder } from "../lib/wallet/stream";

/** 链配置里那份 StreamEscrow 上的分期订单才读链。 */
function onStream(o: OrderView | null | undefined): o is OrderView {
  return (
    !!o && !isOnce(o) && o.contract.toLowerCase() === chain.contracts.StreamEscrow.toLowerCase()
  );
}

function streamQueryKey(o: OrderView) {
  return ["stream", o.contract.toLowerCase(), o.chain_order_id, orderVersion(o)] as const;
}

/**
 * 分期订单的链上状态（净本金、已领取、上次领取时间、领取 gas 费不在 indexer 里）。indexer 反映出变化
 * （订单版本变了）才重新读；解锁进度按当前时间在本地算，不轮询链。
 */
export function useStreamOrder(order: OrderView | null | undefined) {
  const target = onStream(order) ? order : null;
  return useQuery({
    queryKey: target ? streamQueryKey(target) : ["stream", "none"],
    queryFn: async (): Promise<StreamOrder> => {
      if (!target) throw new Error("不是分期担保订单");
      return (await loadWalletSdk()).streamOrder(BigInt(target.chain_order_id));
    },
    enabled: target !== null,
    staleTime: Infinity,
  });
}

const atIso = (secs: bigint) => new Date(Number(secs) * 1000).toISOString();

/** 分期进度（共享的 StreamProgress 组件）+ 卖家可领取的说明；买家只看进度。 */
export function StreamPanel({
  chainOrder,
  action,
  now,
}: {
  chainOrder: StreamOrder;
  action: OrderAction | null;
  now: number;
}) {
  const p = streamProgress(chainOrder, BigInt(Math.floor(now / 1000)));
  const claim = action?.kind === "claim" ? action.claim : null;
  return (
    <div className="space-y-2">
      <StreamProgress
        periodLabel={periodLabel(chainOrder.interval)}
        periods={chainOrder.periods}
        elapsed={p.elapsed}
        net={chainOrder.net.toString()}
        unlocked={p.unlocked.toString()}
        claimed={chainOrder.claimed.toString()}
        disputed={chainOrder.state === StreamState.Disputed}
      />
      {claim ? (
        <p className="text-sm text-fg-muted" data-testid="stream-claimable">
          可领取 <Amount value={p.claimable} />
          {claim.ok ? (
            <>
              ，这次实收 <Amount value={claim.payout} />
              {claim.settles ? "（全部期数已到期，领取即结清订单）" : "（已扣领取 gas 费）"}
            </>
          ) : (
            <>
              ，{claim.reason}
              {claim.retryAt ? `，下次可领取${remaining(atIso(claim.retryAt), now)}` : ""}
            </>
          )}
        </p>
      ) : null}
    </div>
  );
}
