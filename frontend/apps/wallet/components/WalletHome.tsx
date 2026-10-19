"use client";

import { Amount, Card, CardTitle, ErrorState, LoadingState, NetworkBadge } from "@akari/ui";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { formatEther } from "viem";
import { userMessage, walletApi } from "../lib/wallet/api";
import { NETWORK_LABEL } from "../lib/wallet/config";
import type { Wallet } from "../lib/wallet/sdk";
import { useNow } from "../lib/useNow";
import { DepositCard } from "./DepositCard";
import { WalletGate } from "./WalletGate";
import { Withdraw } from "./Withdraw";
import { WithdrawAddresses } from "./WithdrawAddresses";

const BALANCES_KEY = ["balances"] as const;
const ADDRESSES_KEY = ["withdraw-addresses"] as const;

function Dashboard({ wallet }: { wallet: Wallet }) {
  const queryClient = useQueryClient();
  const now = useNow();
  const api = walletApi();
  const balances = useQuery({ queryKey: BALANCES_KEY, queryFn: () => api.balances() });
  const addresses = useQuery({ queryKey: ADDRESSES_KEY, queryFn: () => api.withdrawAddresses() });
  const usdt = balances.data ? BigInt(balances.data.usdt) : null;
  const eth = balances.data ? BigInt(balances.data.eth) : 0n;
  const credits = balances.data
    ? { left: balances.data.withdraw_credits, today: balances.data.withdraws_left_today }
    : null;

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card className="lg:col-span-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-sm text-fg-muted">可用余额</p>
          <NetworkBadge label={NETWORK_LABEL} />
        </div>
        <p className="mt-1 text-4xl font-semibold" data-testid="balance-usdt">
          {usdt === null ? balances.isError ? "—" : "读取中…" : <Amount value={usdt} />}
        </p>
        {balances.isError ? (
          <p role="alert" className="mt-1 text-sm text-warning">
            {userMessage(balances.error, "余额读取失败，请稍后刷新")}
          </p>
        ) : null}
        {eth > 0n ? (
          <p className="amount mt-1 text-sm text-fg-muted">另有 {formatEther(eth)} ETH</p>
        ) : null}
        <p className="mt-1 text-sm text-fg-muted">
          钱包不需要 ETH，gas 由平台代付。建议交易完成后提回交易所，不在钱包长期存放。
        </p>
      </Card>
      <DepositCard address={wallet.address} />
      <Card>
        <CardTitle className="mb-3">提回交易所</CardTitle>
        {addresses.isPending ? (
          <LoadingState rows={2} label="正在读取提币地址" />
        ) : addresses.isError ? (
          <ErrorState message={userMessage(addresses.error, "提币地址读取失败，请稍后刷新")} />
        ) : (
          <Withdraw
            wallet={wallet}
            balance={usdt}
            credits={credits}
            addresses={addresses.data}
            now={now}
            onWithdrawn={() => queryClient.invalidateQueries({ queryKey: BALANCES_KEY })}
          />
        )}
      </Card>
      <Card className="lg:col-span-2">
        <CardTitle className="mb-1">提币地址</CardTitle>
        {addresses.data ? (
          <WithdrawAddresses
            self={wallet.address}
            addresses={addresses.data}
            now={now}
            onChanged={() => queryClient.invalidateQueries({ queryKey: ADDRESSES_KEY })}
          />
        ) : null}
      </Card>
      <p className="text-sm text-fg-muted lg:col-span-2">
        论坛打不开、平台代付停了？钱一直在你自己的钱包里，可以用
        <a href="/emergency" className="text-accent underline">
          紧急取回
        </a>
        页面直接连公共节点取回（需自备少量 ETH 付 gas）。
      </p>
    </div>
  );
}

/** 钱包首页：余额、充值、提回交易所与提币地址；未开通时由 WalletGate 引导。 */
export function WalletHome() {
  return <WalletGate>{(wallet) => <Dashboard wallet={wallet} />}</WalletGate>;
}
