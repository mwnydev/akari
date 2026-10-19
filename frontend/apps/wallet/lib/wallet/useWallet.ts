"use client";

import { ApiError } from "@akari/api-client";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { userMessage, walletApi, type WalletOverview } from "./api";
import { loadWalletSdk } from "./load";
import type { Wallet } from "./sdk";

export type WalletState =
  | { status: "loading" }
  /** 没有论坛会话：钱包不做登录，去论坛登录 */
  | { status: "signed-out" }
  /** 还没开通交易（或 Passkey 注册了但地址没登记上） */
  | { status: "setup"; overview: WalletOverview }
  | { status: "ready"; wallet: Wallet }
  | { status: "error"; message: string };

type Loaded = Exclude<WalletState, { status: "loading" | "error" }>;

const WALLET_KEY = ["wallet"] as const;

async function loadWallet(): Promise<Loaded> {
  const api = walletApi();
  let overview: WalletOverview;
  try {
    overview = await api.overview();
  } catch (e) {
    if (e instanceof ApiError && e.code === "UNAUTHORIZED") return { status: "signed-out" };
    throw e;
  }
  if (!overview.enabled) return { status: "setup", overview };
  // 只有已开通的用户才加载钱包 SDK（不进首屏 JS），按登记的 Passkey 公钥与版本算出账户，不触发 Face ID
  const sdk = await loadWalletSdk();
  return { status: "ready", wallet: await sdk.openWallet(api, overview) };
}

/** 当前用户的钱包；同一页面里多处使用共享一次加载。 */
export function useWallet() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: WALLET_KEY,
    queryFn: loadWallet,
    staleTime: Infinity,
    retry: false,
    // Wallet 对象带方法，不能做结构共享
    structuralSharing: false,
  });

  const state: WalletState = query.isPending
    ? { status: "loading" }
    : query.isError
      ? { status: "error", message: userMessage(query.error, "钱包加载失败，请刷新重试") }
      : query.data;

  /** 开通完成后直接换成已就绪的钱包，不再请求一遍。 */
  const setReady = useCallback(
    (wallet: Wallet) => {
      const ready: Loaded = { status: "ready", wallet };
      queryClient.setQueryData(WALLET_KEY, ready);
    },
    [queryClient],
  );
  return { state, setReady };
}
