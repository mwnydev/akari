"use client";

import { Button, Card, EmptyState, ErrorState, LoadingState } from "@akari/ui";
import type { ReactNode } from "react";
import type { Wallet } from "../lib/wallet/sdk";
import { useWallet } from "../lib/wallet/useWallet";

/** 需要钱包的页面统一处理加载 / 未开通 / 出错，开通后才渲染内容。 */
export function WalletGate({ children }: { children: (wallet: Wallet) => ReactNode }) {
  const { state } = useWallet();
  switch (state.status) {
    case "loading":
      return <LoadingState rows={2} label="正在读取钱包" />;
    case "error":
      return <ErrorState message={state.message} />;
    case "none":
      return (
        <Card>
          <EmptyState
            title="还没有开通交易"
            description="开通后才能签报价、付款和确认收货。"
            action={
              <Button asChild>
                <a href="/setup">去开通</a>
              </Button>
            }
          />
        </Card>
      );
    case "ready":
      return children(state.wallet);
  }
}
