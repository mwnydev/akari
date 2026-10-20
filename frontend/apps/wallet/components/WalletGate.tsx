"use client";

import { Button, Card, EmptyState, ErrorState, LoadingState } from "@akari/ui";
import { useSyncExternalStore, type ReactNode } from "react";
import { FORUM_ORIGIN } from "../lib/wallet/config";
import type { Wallet } from "../lib/wallet/sdk";
import { useWallet } from "../lib/wallet/useWallet";

const noSubscribe = () => () => undefined;

/**
 * 去论坛的某个页面，处理完回到钱包当前页面（论坛只接受配置的钱包源，见论坛 lib/safe-next.ts）。
 * 地址在浏览器里补上（静态导出的页面在构建时不知道查询参数），服务端渲染先用不带 next 的地址。
 */
export function useForumHref(path: string): string {
  const here = useSyncExternalStore(
    noSubscribe,
    () => location.href,
    () => null,
  );
  return here
    ? `${FORUM_ORIGIN}${path}?next=${encodeURIComponent(here)}`
    : `${FORUM_ORIGIN}${path}`;
}

/** 钱包没有自己的登录：用论坛账号（会话 Cookie 与论坛共用，ADR 0009），登录后回到这里。 */
export function LoginPrompt() {
  const href = useForumHref("/login");
  return (
    <Card>
      <EmptyState
        title="请先登录论坛"
        description="钱包使用论坛账号，登录后会回到这个页面继续。"
        action={
          <Button asChild>
            <a href={href}>去登录</a>
          </Button>
        }
      />
    </Card>
  );
}

/** 需要钱包的页面统一处理加载 / 未登录 / 未开通 / 出错，开通后才渲染内容。 */
export function WalletGate({ children }: { children: (wallet: Wallet) => ReactNode }) {
  const { state } = useWallet();
  switch (state.status) {
    case "loading":
      return <LoadingState rows={2} label="正在读取钱包" />;
    case "error":
      return <ErrorState message={state.message} />;
    case "signed-out":
      return <LoginPrompt />;
    case "setup":
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
