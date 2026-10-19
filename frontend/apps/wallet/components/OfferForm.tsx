"use client";

import {
  Amount,
  Button,
  Card,
  CardTitle,
  ErrorState,
  Field,
  Input,
  LoadingState,
  shortAddress,
} from "@akari/ui";
import { SignConfirmDialog, type ConfirmRow } from "@akari/ui/sign-confirm";
import { useQuery } from "@tanstack/react-query";
import { CircleCheck, PenLine } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import type { Address, Hex } from "viem";
import { parseUsdt } from "../lib/amount";
import { userMessage, walletApi, type ListingView, type TradeParams } from "../lib/wallet/api";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import type { OfferDraft, OfferTarget, Wallet } from "../lib/wallet/sdk";
import { WalletGate } from "./WalletGate";

const DAY = 86_400;
const isAddress = (s: string): s is Address => /^0x[0-9a-fA-F]{40}$/.test(s);
const FEE_PAYER_TEXT = ["卖家", "买家", "双方各半"];

function intIn(raw: string, min: number, max: number): number | null {
  const n = Number(raw);
  return Number.isInteger(n) && n >= min && n <= max ? n : null;
}

function useTradeParams() {
  return useQuery({
    queryKey: ["trade-params"],
    queryFn: () => walletApi().tradeParams(),
    staleTime: 5 * 60_000,
  });
}

function OfferSent({ id, onAgain }: { id: number; onAgain: () => void }) {
  const link = `${location.origin}/pay?offer=${id}`;
  return (
    <Card className="space-y-3">
      <p className="flex items-center gap-2 font-medium text-success" data-testid="offer-verified">
        <CircleCheck size={20} aria-hidden />
        报价已签名、通过链上验证，并已发出
      </p>
      <p className="text-sm text-fg-muted">
        买家在报价卡片上点付款即可；也可以把付款链接发给对方：
      </p>
      <a
        data-testid="offer-link"
        href={link}
        className="block font-mono text-sm break-all text-accent"
      >
        {link}
      </a>
      <Button variant="secondary" onClick={onAgain}>
        再签一份
      </Button>
    </Card>
  );
}

/** 签名并发出报价：确认页 →（需要时）Passkey 再验证 → Passkey 签 EIP-712 → 链上验签 → 提交报价卡片。 */
function SignOffer({
  wallet,
  target,
  draft,
  rows,
}: {
  wallet: Wallet;
  target: OfferTarget;
  /** null = 表单还没填好 */
  draft: OfferDraft | null;
  rows: ConfirmRow[];
}) {
  const [open, setOpen] = useState(false);
  const [sent, setSent] = useState<number | null>(null);

  if (sent !== null) return <OfferSent id={sent} onAgain={() => setSent(null)} />;
  return (
    <>
      <Button className="w-full" disabled={!draft} onClick={() => setOpen(true)}>
        <PenLine size={18} aria-hidden />
        签名报价
      </Button>
      {draft ? (
        <SignConfirmDialog
          open={open}
          onOpenChange={setOpen}
          action="签署担保报价"
          amount={draft.amount.toString()}
          to={chain.contracts.Escrow}
          toLabel="担保合约"
          network={NETWORK_LABEL}
          rows={rows}
          note="签名不会动用你的钱，只是授权买家按这些条款付款建单。发出前可能需要先用 Passkey 确认一次身份。"
          onConfirm={async () => {
            const sdk = await loadWalletSdk();
            const id = await sdk.publishOffer(walletApi(), wallet, draft, target);
            setOpen(false);
            setSent(id);
          }}
        />
      ) : null}
    </>
  );
}

/** 交易帖下的公开报价：价格与条款以交易帖为准（listingHash = 帖子内容哈希），卖家只确认签名。 */
function ListingOffer({
  wallet,
  listingId,
  params,
}: {
  wallet: Wallet;
  listingId: number;
  params: TradeParams;
}) {
  const listing = useQuery({
    queryKey: ["listing", listingId],
    queryFn: () => walletApi().listing(listingId),
  });
  if (listing.isPending) return <LoadingState rows={2} label="正在读取交易帖" />;
  if (listing.isError) return <ErrorState message={userMessage(listing.error, "交易帖读取失败")} />;
  const l: ListingView = listing.data;
  const t = l.terms;
  const draft: OfferDraft = {
    amount: BigInt(l.price),
    subject: { listingHash: l.content_hash as Hex },
    feePayer: t.fee_payer ?? 0,
    bond: BigInt(t.bond ?? "0"),
    lateSlashBps: t.late_slash_bps ?? 0,
    instant: t.instant ?? false,
    shipWindow: t.ship_window,
    confirmWindow: t.confirm_window,
    ttl: params.defaults.public_offer_ttl,
  };
  const windows = `${t.ship_window / DAY} 天 / ${t.confirm_window / DAY} 天`;
  return (
    <Card className="space-y-3">
      <CardTitle>为交易帖签公开报价</CardTitle>
      <dl className="space-y-2 text-sm">
        <div className="flex justify-between gap-3">
          <dt className="text-fg-muted">价格</dt>
          <dd>
            <Amount value={l.price} />
          </dd>
        </div>
        <div className="flex justify-between gap-3">
          <dt className="text-fg-muted">发货期 / 确认期</dt>
          <dd>{windows}</dd>
        </div>
        <div className="flex justify-between gap-3">
          <dt className="text-fg-muted">手续费承担</dt>
          <dd>{FEE_PAYER_TEXT[t.fee_payer ?? 0]}</dd>
        </div>
      </dl>
      <p className="text-sm text-fg-muted">
        公开报价 {Math.round(params.defaults.public_offer_ttl / DAY)} 天内有效，只能成交一次。
      </p>
      <SignOffer
        wallet={wallet}
        target={{ listing_id: listingId }}
        draft={draft}
        rows={[
          { label: "买家", value: "任何人" },
          { label: "发货期 / 确认期", value: windows },
        ]}
      />
    </Card>
  );
}

/** 私信里的报价：只给对方这位买家（后端要求指定买家），条款在会话里谈好后填写。 */
function ConversationOffer({
  wallet,
  conversationId,
  params,
}: {
  wallet: Wallet;
  conversationId: number;
  params: TradeParams;
}) {
  const [title, setTitle] = useState("");
  const [rawAmount, setRawAmount] = useState("");
  const [buyer, setBuyer] = useState("");
  const [shipDays, setShipDays] = useState(String(params.defaults.ship_window / DAY));
  const [confirmDays, setConfirmDays] = useState(String(params.defaults.confirm_window / DAY));

  const minDays = Math.ceil(params.min_window / DAY);
  const maxDays = Math.floor(params.max_window / DAY);
  const amount = parseUsdt(rawAmount);
  const ship = intIn(shipDays, minDays, maxDays);
  const confirm = intIn(confirmDays, minDays, maxDays);
  const buyerOk = isAddress(buyer) && buyer.toLowerCase() !== wallet.address.toLowerCase();
  const valid = title.trim() !== "" && amount !== null && amount > 0n && ship && confirm && buyerOk;
  const range = `${minDays}–${maxDays} 天`;

  const draft: OfferDraft | null = valid
    ? {
        buyer,
        amount,
        subject: { title: title.trim() },
        feePayer: 0, // 卖家承担手续费（plan 第 6 节默认）
        bond: 0n,
        lateSlashBps: 0,
        instant: false,
        shipWindow: ship * DAY,
        confirmWindow: confirm * DAY,
        ttl: params.defaults.private_offer_ttl,
      }
    : null;

  return (
    <Card className="space-y-3">
      <CardTitle>发起担保报价</CardTitle>
      <Field label="商品 / 服务">
        <Input name="title" value={title} onChange={(e) => setTitle(e.target.value)} />
      </Field>
      <Field
        label="金额（USDT）"
        error={rawAmount && amount === null ? "最多 6 位小数" : undefined}
      >
        <Input
          name="amount"
          inputMode="decimal"
          className="amount"
          value={rawAmount}
          onChange={(e) => setRawAmount(e.target.value)}
        />
      </Field>
      <Field
        label="买家钱包地址"
        hint="私信里的报价只能由这位买家付款"
        error={buyer && !buyerOk ? "地址格式不对，或者填成了你自己的地址" : undefined}
      >
        <Input
          name="buyer"
          className="font-mono"
          value={buyer}
          onChange={(e) => setBuyer(e.target.value.trim())}
        />
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field label="发货期（天）" error={ship ? undefined : range}>
          <Input
            name="shipDays"
            inputMode="numeric"
            value={shipDays}
            onChange={(e) => setShipDays(e.target.value)}
          />
        </Field>
        <Field label="确认期（天）" error={confirm ? undefined : range}>
          <Input
            name="confirmDays"
            inputMode="numeric"
            value={confirmDays}
            onChange={(e) => setConfirmDays(e.target.value)}
          />
        </Field>
      </div>
      <p className="text-sm text-fg-muted">
        手续费由卖家承担；报价 {Math.round(params.defaults.private_offer_ttl / 3600)}{" "}
        小时内有效，只能成交一次。
      </p>
      <SignOffer
        wallet={wallet}
        target={{ conversation_id: conversationId }}
        draft={draft}
        rows={[
          { label: "买家", value: buyerOk ? shortAddress(buyer) : "" },
          { label: "发货期 / 确认期", value: `${ship} 天 / ${confirm} 天` },
        ]}
      />
    </Card>
  );
}

function OfferPanel({ wallet, target }: { wallet: Wallet; target: OfferTarget }) {
  const params = useTradeParams();
  if (params.isPending) return <LoadingState rows={2} label="正在读取交易参数" />;
  if (params.isError) return <ErrorState message={userMessage(params.error, "交易参数读取失败")} />;
  return "listing_id" in target ? (
    <ListingOffer wallet={wallet} listingId={target.listing_id} params={params.data} />
  ) : (
    <ConversationOffer
      wallet={wallet}
      conversationId={target.conversation_id}
      params={params.data}
    />
  );
}

const ID = /^[1-9]\d{0,15}$/;

/** `/offer?listing=…`（交易帖）或 `/offer?conversation=…`（私信会话）：报价卡片必须挂在其中一处。 */
export function OfferSection() {
  const q = useSearchParams();
  const listing = q.get("listing");
  const conversation = q.get("conversation");
  const target: OfferTarget | null =
    listing && ID.test(listing)
      ? { listing_id: Number(listing) }
      : conversation && ID.test(conversation)
        ? { conversation_id: Number(conversation) }
        : null;
  if (!target) {
    return (
      <ErrorState
        title="请从交易帖或私信发起担保"
        message="报价卡片要挂在交易帖下或发在私信会话里，请从对应页面的「发起担保」进入"
      />
    );
  }
  return <WalletGate>{(wallet) => <OfferPanel wallet={wallet} target={target} />}</WalletGate>;
}
