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
import { CircleCheck, ScanFace } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import { decodeSignedOffer, type SignedOffer } from "../lib/wallet/offer";
import type { CheckoutQuote, Wallet } from "../lib/wallet/sdk";
import { WalletGate } from "./WalletGate";

const DAY = 86_400;

type Check =
  | { status: "loading" }
  | { status: "invalid"; message: string }
  | { status: "ok"; quote: CheckoutQuote; balance: bigint };

/** 买家：核对卖家签名与费用 → 确认页 → 一次 Face ID 完成授权 + 付款建单（一个 UserOperation）。 */
export function PayOffer({ signed, wallet }: { signed: SignedOffer; wallet: Wallet }) {
  const { offer } = signed;
  const [check, setCheck] = useState<Check>({ status: "loading" });
  const [open, setOpen] = useState(false);
  const [orderId, setOrderId] = useState<bigint | null>(null);

  useEffect(() => {
    let live = true;
    (async () => {
      const sdk = await loadWalletSdk();
      if (
        !/^0x0{40}$/.test(offer.buyer) &&
        offer.buyer.toLowerCase() !== wallet.address.toLowerCase()
      ) {
        return { status: "invalid", message: "这份报价指定了别的买家" } as const;
      }
      if (!(await sdk.verifyOffer(signed)))
        return { status: "invalid", message: "卖家签名无效，请不要付款" } as const;
      const [quote, balance] = await Promise.all([sdk.quoteCheckout(offer), wallet.balance()]);
      return { status: "ok", quote, balance } as const;
    })().then(
      (c) => live && setCheck(c),
      (e: unknown) =>
        live &&
        setCheck({ status: "invalid", message: e instanceof Error ? e.message : "报价校验失败" }),
    );
    return () => {
      live = false;
    };
  }, [signed, offer, wallet]);

  if (orderId !== null) {
    return (
      <Card className="space-y-3">
        <p className="flex items-center gap-2 font-medium text-success">
          <CircleCheck size={20} aria-hidden />
          付款成功，订单 #{orderId.toString()} 已建立
        </p>
        <Button asChild>
          <a data-testid="order-link" href={`/order?id=${orderId}`}>
            查看订单
          </a>
        </Button>
      </Card>
    );
  }
  if (check.status === "loading") return <LoadingState rows={2} label="正在核对报价" />;
  if (check.status === "invalid") return <ErrorState title="不能付款" message={check.message} />;

  const { quote, balance } = check;
  const enough = balance >= quote.total;
  return (
    <Card className="space-y-3">
      <CardTitle>确认下单</CardTitle>
      <dl className="space-y-2 text-sm">
        {[
          {
            label: "卖家",
            value: <span className="amount font-mono">{shortAddress(offer.seller)}</span>,
          },
          {
            label: "发货期 / 确认期",
            value: `${offer.shipWindow / DAY} 天 / ${offer.confirmWindow / DAY} 天`,
          },
          { label: "货款", value: <Amount value={offer.amount} /> },
          {
            label: "你承担的手续费与 gas 费",
            value: <Amount value={quote.buyerPays - offer.amount} />,
          },
          { label: "代付费", value: <Amount value={quote.sponsorFee} /> },
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
        <p className="rounded-lg bg-warning-soft p-3 text-sm text-warning">
          余额不足，请先从交易所选 Arbitrum One 网络提币到钱包。
        </p>
      )}
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action="担保付款"
        amount={quote.total.toString()}
        to={chain.contracts.Escrow}
        toLabel="担保合约"
        network={NETWORK_LABEL}
        rows={[
          { label: "货款（托管至确认收货）", value: <Amount value={offer.amount} /> },
          { label: "代付费", value: <Amount value={quote.sponsorFee} /> },
        ]}
        note="授权与付款合成一次 Face ID。付款成功的那一刻订单才成立；余额不足时整笔不会上链，也不扣代付费。"
        onConfirm={async () => {
          const { orderId: id } = await wallet.pay(signed, quote);
          setOpen(false);
          setOrderId(id);
        }}
      />
    </Card>
  );
}

/** `/pay?offer=…`：解码付款链接里的报价（页面需包在 Suspense 里，静态导出读查询参数的要求）。 */
export function PaySection() {
  const code = useSearchParams().get("offer");
  const signed = useMemo(() => (code ? decodeSignedOffer(code) : null), [code]);
  if (!signed)
    return <ErrorState title="报价链接无效" message="链接不完整或被改动过，请向卖家重新索取" />;
  return <WalletGate>{(wallet) => <PayOffer signed={signed} wallet={wallet} />}</WalletGate>;
}
