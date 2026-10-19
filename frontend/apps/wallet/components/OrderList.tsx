"use client";

import { Amount, Badge, Button, Card, EmptyState, ErrorState, LoadingState } from "@akari/ui";
import { SegmentedControl } from "@akari/ui/tabs";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useState } from "react";
import { userMessage, walletApi, type OrderRole } from "../lib/wallet/api";
import { STATE_TEXT, levelBadge, onceState } from "../lib/wallet/order";

const ROLES: { value: OrderRole; label: string }[] = [
  { value: "buyer", label: "我买的" },
  { value: "seller", label: "我卖的" },
];

/** 我的订单（游标分页，plan 第 14 节「列表一律游标分页」）。 */
export function OrderList() {
  const [role, setRole] = useState<OrderRole>("buyer");
  const query = useInfiniteQuery({
    queryKey: ["orders", role],
    queryFn: ({ pageParam }) => walletApi().orders(role, pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const orders = query.data?.pages.flatMap((p) => p.orders) ?? [];

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-2xl font-semibold">我的订单</h1>
        <SegmentedControl label="订单角色" options={ROLES} value={role} onValueChange={setRole} />
      </div>
      {query.isPending ? (
        <LoadingState rows={3} label="正在读取订单" />
      ) : query.isError ? (
        <ErrorState message={userMessage(query.error, "订单读取失败，请稍后刷新")} />
      ) : orders.length === 0 ? (
        <Card>
          <EmptyState title="还没有订单" description="付款成功后订单会出现在这里。" />
        </Card>
      ) : (
        <Card className="p-0 sm:p-0">
          <ul className="divide-y divide-border">
            {orders.map((o) => {
              const state = onceState(o);
              const level = levelBadge(o);
              return (
                <li key={o.id}>
                  <a
                    href={`/order?id=${o.id}`}
                    className="flex items-center justify-between gap-3 px-4 py-3 hover:bg-surface-2 sm:px-5"
                  >
                    <span className="min-w-0">
                      <span className="block font-medium">订单 #{o.id}</span>
                      <span className="block text-sm text-fg-muted">
                        {state ? STATE_TEXT[state] : "已撤销"}
                      </span>
                    </span>
                    <span className="flex shrink-0 flex-col items-end gap-1">
                      <Amount value={o.amount} />
                      <Badge tone={level.tone}>{level.label}</Badge>
                    </span>
                  </a>
                </li>
              );
            })}
          </ul>
        </Card>
      )}
      {query.hasNextPage ? (
        <Button
          variant="secondary"
          className="w-full"
          disabled={query.isFetchingNextPage}
          onClick={() => query.fetchNextPage()}
        >
          {query.isFetchingNextPage ? "加载中…" : "加载更多"}
        </Button>
      ) : null}
    </div>
  );
}
