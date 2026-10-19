"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { api, unwrap } from "../../lib/browser-api";
import { OfferCard } from "../trade/OfferCard";
import { OFFER_CARD_EVENT } from "./preview";

export const offersKey = (conversationId: number) => ["conversation-offers", conversationId];

/**
 * 会话里的报价卡片：说明这次事件（发出、修改、拒绝……），卡片本身显示报价的当前状态。
 * 买家可在钱包里付款或在这里拒绝，卖家可取消；已成交的卡片链到订单页。
 */
export function OfferMessage({
  event,
  offerId,
  listed,
  listReady,
  me,
  conversationId,
}: {
  event: string;
  offerId: number;
  /** 会话报价列表里的这张（列表只有最新 50 张） */
  listed: Schemas["OfferView"] | undefined;
  listReady: boolean;
  me: number;
  conversationId: number;
}) {
  const qc = useQueryClient();
  const single = useQuery({
    queryKey: ["offer", offerId],
    queryFn: () => unwrap(api.GET("/api/v1/offers/{id}", { params: { path: { id: offerId } } })),
    enabled: listReady && !listed,
  });
  const offer = listed ?? single.data;
  const isSeller = offer?.seller_id === me;
  // 指定买家的卡片只有那位买家能付；公开报价（无 buyer_id）除卖家外都能付
  const isBuyer = offer !== undefined && !isSeller && (offer.buyer_id ?? me) === me;
  const order = useQuery({
    queryKey: ["offer-order", offerId],
    queryFn: () =>
      unwrap(api.GET("/api/v1/offers/{id}/order", { params: { path: { id: offerId } } })),
    enabled: offer?.status === "paid",
    staleTime: Infinity,
  });
  const close = useMutation({
    mutationFn: (kind: "reject" | "cancel") => {
      const path = { params: { path: { id: offerId } } };
      return unwrap(
        kind === "reject"
          ? api.POST("/api/v1/offers/{id}/reject", { ...path, body: { reason: null } })
          : api.POST("/api/v1/offers/{id}/cancel", path),
      );
    },
    onSuccess: () =>
      Promise.all([
        qc.invalidateQueries({ queryKey: offersKey(conversationId) }),
        qc.invalidateQueries({ queryKey: ["offer", offerId] }),
      ]),
  });

  const label = OFFER_CARD_EVENT[event] ?? "报价有更新";
  return (
    <div className="w-72 max-w-full space-y-1" data-testid="offer-message">
      <p className="text-xs text-fg-muted">{label}</p>
      {offer ? (
        <OfferCard offer={offer} canPay={isBuyer}>
          {offer.status === "pending" && (isBuyer || isSeller) ? (
            <Button
              variant="secondary"
              size="sm"
              className="w-full"
              disabled={close.isPending}
              onClick={() => close.mutate(isSeller ? "cancel" : "reject")}
            >
              {isSeller ? "取消报价" : "拒绝报价"}
            </Button>
          ) : null}
          {close.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(close.error)}
            </p>
          ) : null}
          {order.data ? (
            <Button asChild variant="secondary" size="sm" className="w-full">
              <Link href={`/orders/${order.data.id}`}>查看订单</Link>
            </Button>
          ) : null}
        </OfferCard>
      ) : (
        <p className="rounded-lg bg-surface-2 px-3 py-2 text-sm text-fg-muted">
          {single.isError ? "报价卡片加载失败" : "报价卡片加载中…"}
        </p>
      )}
    </div>
  );
}
