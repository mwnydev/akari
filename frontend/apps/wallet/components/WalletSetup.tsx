"use client";

import { Amount, Button, Card, CardTitle, ErrorState, LoadingState, NetworkBadge } from "@akari/ui";
import { KeyRound, LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { NETWORK_LABEL } from "../lib/wallet/config";
import type { Wallet } from "../lib/wallet/sdk";
import { useWallet } from "../lib/wallet/useWallet";

function WalletCard({ wallet }: { wallet: Wallet }) {
  const [balance, setBalance] = useState<bigint | null>(null);
  useEffect(() => {
    wallet.balance().then(setBalance, () => setBalance(null));
  }, [wallet]);
  return (
    <Card className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <CardTitle>你的钱包</CardTitle>
        <NetworkBadge label={NETWORK_LABEL} />
      </div>
      <div>
        <p className="text-sm text-fg-muted">钱包地址（充值请转到这里）</p>
        <p data-testid="wallet-address" className="amount font-mono text-sm break-all">
          {wallet.address}
        </p>
      </div>
      <p className="text-sm text-fg-muted">
        余额 {balance === null ? "读取中…" : <Amount value={balance} />}
      </p>
      <p className="text-sm text-fg-muted">
        钱包第一次链上操作时自动部署，地址不会改变。gas 由平台代付，钱包不需要 ETH。
      </p>
    </Card>
  );
}

/** 开通交易：创建钱包 Passkey → 算出智能账户地址（plan 第 5 节）。 */
export function WalletSetup() {
  const { state, create } = useWallet();
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState("");

  if (state.status === "loading") return <LoadingState rows={2} label="正在读取钱包" />;
  if (state.status === "error") return <ErrorState message={state.message} />;
  if (state.status === "ready") return <WalletCard wallet={state.wallet} />;

  return (
    <Card className="space-y-3">
      <CardTitle>开通交易</CardTitle>
      <p className="text-sm text-fg-muted">
        会在这台设备上创建一个「Akari 钱包签名（勿删）」Passkey，只用于确认付款和敏感操作。 iCloud
        钥匙串 / Google 密码管理器会自动同步，请不要删除这条记录。
      </p>
      {error ? (
        <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
          {error}
        </p>
      ) : null}
      <Button
        size="lg"
        className="w-full"
        disabled={creating}
        onClick={async () => {
          setCreating(true);
          setError("");
          try {
            await create();
          } catch (e) {
            setError(e instanceof Error ? e.message : "创建 Passkey 没有完成，请重试");
          } finally {
            setCreating(false);
          }
        }}
      >
        {creating ? (
          <LoaderCircle size={20} className="animate-spin" aria-hidden />
        ) : (
          <KeyRound size={20} aria-hidden />
        )}
        创建钱包 Passkey
      </Button>
    </Card>
  );
}
