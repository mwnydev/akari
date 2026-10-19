"use client";

import { useCallback, useEffect, useState } from "react";
import { walletApi } from "./api";
import { loadWalletSdk } from "./load";
import type { Wallet } from "./sdk";

export type WalletState =
  | { status: "loading" }
  | { status: "none" }
  | { status: "ready"; wallet: Wallet }
  | { status: "error"; message: string };

/** 当前用户的钱包：挂载后再按需加载 SDK（不进首屏 JS），按登记的 Passkey 公钥算出账户，不触发 Face ID。 */
export function useWallet() {
  const [state, setState] = useState<WalletState>({ status: "loading" });

  useEffect(() => {
    let live = true;
    loadWalletSdk()
      .then((sdk) => sdk.openWallet(walletApi()))
      .then((wallet) => {
        if (live) setState(wallet ? { status: "ready", wallet } : { status: "none" });
      })
      .catch((e: unknown) => {
        if (live)
          setState({ status: "error", message: e instanceof Error ? e.message : "钱包加载失败" });
      });
    return () => {
      live = false;
    };
  }, []);

  /** 开通交易：创建 Passkey（一次 Face ID）并算出地址；错误交给调用方显示。 */
  const create = useCallback(async () => {
    const sdk = await loadWalletSdk();
    const wallet = await sdk.createWallet(walletApi());
    setState({ status: "ready", wallet });
  }, []);

  return { state, create };
}
