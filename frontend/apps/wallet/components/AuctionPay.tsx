"use client";

import {
  Amount,
  Button,
  Card,
  CardTitle,
  ErrorState,
  LoadingState,
  NetworkBadge,
  shortAddress,
} from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { useQuery } from "@tanstack/react-query";
import { CircleCheck, LoaderCircle, ScanFace } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { formatCnyPerUsdt } from "../lib/cny";
import { userMessage, walletApi } from "../lib/wallet/api";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import type { AuctionCheckout, Wallet } from "../lib/wallet/sdk";
import { AUCTION_ID, AuctionPrice, windowsText } from "./AuctionTerms";
import { DepositCard } from "./DepositCard";
import { WalletGate } from "./WalletGate";

/** 付款上链后等 indexer 同步出订单的轮询间隔。 */
const SYNC_POLL_MS = 2000;

/** 付款已上链：等 indexer 把拍卖标为已成交（带上站内订单 ID）后给出订单链接。 */
function Paid({ auctionId }: { auctionId: number }) {
  const order = useQuery({
    queryKey: ["auction-order", auctionId],
    queryFn: async () => (await walletApi().auction(auctionId)).order_id ?? null,
    refetchInterval: (q) => (q.state.data ? false : SYNC_POLL_MS),
  });
  return (
    <Card className="space-y-3">
      <p className="flex items-center gap-2 font-medium text-success" role="status">
        <CircleCheck size={20} aria-hidden />
        付款成功{order.data ? `，订单 #${order.data} 已建立` : ""}
      </p>
      {order.data ? (
        <Button asChild>
          <a data-testid="order-link" href={`/order?id=${order.data}`}>
            查看订单
          </a>
        </Button>
      ) : order.isError ? (
        <p role="alert" className="text-sm text-warning">
          {userMessage(order.error, "订单读取失败")}，稍后可在「我的订单」里查看
        </p>
      ) : (
        <p className="flex items-center gap-2 text-sm text-fg-muted">
          <LoaderCircle size={16} className="animate-spin" aria-hidden />
          付款已上链，正在同步订单…
        </p>
      )}
    </Card>
  );
}

function Confirm({
  checkout: c,
  wallet,
  onPaid,
}: {
  checkout: AuctionCheckout;
  wallet: Wallet;
  onPaid: () => void;
}) {
  const [open, setOpen] = useState(false);
  const { auth, cny, proof, amount, quote, balance } = c;
  const enough = balance >= quote.total;
  const deadline = new Date(Number(proof.payDeadline) * 1000).toLocaleString("zh-CN", {
    hour12: false,
  });
  return (
    <Card className="space-y-3">
      <CardTitle>拍卖成交 · 付款建单</CardTitle>
      {/* ADR 0011：付款页同时显示人民币成交价、卖家汇率与应付 USDT */}
      <p className="amount text-sm" data-testid="auction-settlement">
        成交价 <AuctionPrice cny={cny} amount={proof.price} />
        {cny ? ` · 卖家汇率 ${formatCnyPerUsdt(auth.cnyPerUsdt)} · 折合 ` : " · 货款 "}
        <Amount value={amount} />
      </p>
      <dl className="space-y-2 text-sm">
        {[
          {
            label: "卖家",
            value: <span className="amount font-mono">{shortAddress(auth.seller)}</span>,
          },
          {
            label: "发货期 / 确认期",
            value: windowsText(auth),
          },
          { label: "货款", value: <Amount value={amount} /> },
          {
            label: "你承担的手续费与 gas 费",
            value: <Amount value={quote.buyerPays - amount} />,
          },
          { label: "代付费", value: <Amount value={quote.sponsorFee} /> },
          { label: "付款截止", value: <span className="amount">{deadline}</span> },
        ].map((r) => (
          <div key={r.label} className="flex justify-between gap-3">
            <dt className="text-fg-muted">{r.label}</dt>
            <dd>{r.value}</dd>
          </div>
        ))}
        <div className="flex justify-between gap-3 border-t border-border pt-2 text-lg font-semibold">
          <dt>合计</dt>
          <dd data-testid="pay-total">
            <Amount value={quote.total} />
          </dd>
        </div>
      </dl>
      <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
        <span className="text-fg-muted">
          钱包余额 <Amount value={balance} />
        </span>
        <NetworkBadge label={NETWORK_LABEL} />
      </div>
      {enough ? (
        <Button size="lg" className="w-full" onClick={() => setOpen(true)}>
          <ScanFace size={20} aria-hidden />
          付款 <Amount value={quote.total} />
        </Button>
      ) : (
        <>
          <p role="alert" className="rounded-lg bg-warning-soft p-3 text-sm text-warning">
            余额不足：还差 <Amount value={quote.total - balance} />
            。请先从交易所选 Arbitrum One 网络提币到下面的地址，到账后刷新本页。
          </p>
          <DepositCard address={wallet.address} />
        </>
      )}
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action="拍卖成交付款"
        amount={quote.total.toString()}
        to={chain.contracts.Escrow}
        toLabel="担保合约"
        network={NETWORK_LABEL}
        rows={[
          { label: "成交价", value: <AuctionPrice cny={cny} amount={proof.price} /> },
          { label: "货款（托管至确认收货）", value: <Amount value={amount} /> },
          { label: "代付费", value: <Amount value={quote.sponsorFee} /> },
        ]}
        note="授权与付款合成一次 Face ID。付款成功的那一刻订单才成立；余额不足时整笔不会上链，也不扣代付费。"
        onConfirm={async () => {
          await wallet.payAuction(c);
          setOpen(false);
          onPaid();
        }}
      />
    </Card>
  );
}

/** 赢家：后端给出付款数据 → 核对网络、赢家、两份签名、汇率换算与费用 → 确认页 → 一次 Face ID 付款建单。 */
function AuctionPay({ auctionId, wallet }: { auctionId: number; wallet: Wallet }) {
  const [paid, setPaid] = useState(false);
  const checkout = useQuery({
    queryKey: ["auction-checkout", auctionId, wallet.address],
    queryFn: async () =>
      (await loadWalletSdk()).prepareAuctionCheckout(walletApi(), wallet, auctionId),
    retry: false,
    staleTime: Infinity,
    structuralSharing: false,
  });
  if (paid) return <Paid auctionId={auctionId} />;
  if (checkout.isPending) return <LoadingState rows={2} label="正在核对成交数据" />;
  if (checkout.isError) {
    return (
      <ErrorState
        title="不能付款"
        message={userMessage(checkout.error, "成交数据校验失败，请稍后再试")}
      />
    );
  }
  return <Confirm checkout={checkout.data} wallet={wallet} onPaid={() => setPaid(true)} />;
}

/** `/auction/pay?auction=<拍卖编号>`：拍卖赢家（或递补者）的付款页。 */
export function AuctionPaySection() {
  const id = useSearchParams().get("auction");
  if (!id || !AUCTION_ID.test(id)) {
    return (
      <ErrorState title="付款链接无效" message="链接不完整，请从论坛的拍卖帖点「去付款」进入" />
    );
  }
  return (
    <WalletGate>{(wallet) => <AuctionPay auctionId={Number(id)} wallet={wallet} />}</WalletGate>
  );
}
