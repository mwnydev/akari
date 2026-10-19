"use client";

import { Button, Card, CardTitle, Field, Input, shortAddress } from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { CircleCheck, PenLine } from "lucide-react";
import { useState } from "react";
import type { Address } from "viem";
import { parseUsdt } from "../lib/amount";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import { encodeSignedOffer } from "../lib/wallet/offer";
import type { Wallet } from "../lib/wallet/sdk";
import { WalletGate } from "./WalletGate";

const isAddress = (s: string): s is Address => /^0x[0-9a-fA-F]{40}$/.test(s);

function intIn(raw: string, min: number, max: number): number | null {
  const n = Number(raw);
  return Number.isInteger(n) && n >= min && n <= max ? n : null;
}

/** 卖家：填条款 → 确认页 → Passkey 签 EIP-712 报价 → 链上验签 → 付款链接（之后由私信报价卡片承载）。 */
export function OfferForm({ wallet }: { wallet: Wallet }) {
  const [title, setTitle] = useState("");
  const [rawAmount, setRawAmount] = useState("");
  const [buyer, setBuyer] = useState("");
  const [shipDays, setShipDays] = useState("3");
  const [confirmDays, setConfirmDays] = useState("3");
  const [open, setOpen] = useState(false);
  const [link, setLink] = useState("");

  const amount = parseUsdt(rawAmount);
  const ship = intIn(shipDays, 1, 30);
  const confirm = intIn(confirmDays, 1, 30);
  const buyerOk = buyer === "" || isAddress(buyer);
  const valid = title.trim() !== "" && amount !== null && amount > 0n && ship && confirm && buyerOk;

  if (link) {
    return (
      <Card className="space-y-3">
        <p
          className="flex items-center gap-2 font-medium text-success"
          data-testid="offer-verified"
        >
          <CircleCheck size={20} aria-hidden />
          报价已签名，并已在链上验证通过
        </p>
        <p className="text-sm text-fg-muted">
          把付款链接发给买家（上线后由私信里的报价卡片承载）：
        </p>
        <a
          data-testid="offer-link"
          href={link}
          className="block font-mono text-sm break-all text-accent"
        >
          {link}
        </a>
        <Button variant="secondary" onClick={() => setLink("")}>
          再签一份
        </Button>
      </Card>
    );
  }

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
        hint="留空 = 任何人都能按这份报价付款"
        error={buyerOk ? undefined : "地址格式不对"}
      >
        <Input
          name="buyer"
          className="font-mono"
          value={buyer}
          onChange={(e) => setBuyer(e.target.value.trim())}
        />
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field label="发货期（天）" error={ship ? undefined : "1–30 天"}>
          <Input
            name="shipDays"
            inputMode="numeric"
            value={shipDays}
            onChange={(e) => setShipDays(e.target.value)}
          />
        </Field>
        <Field label="确认期（天）" error={confirm ? undefined : "1–30 天"}>
          <Input
            name="confirmDays"
            inputMode="numeric"
            value={confirmDays}
            onChange={(e) => setConfirmDays(e.target.value)}
          />
        </Field>
      </div>
      <p className="text-sm text-fg-muted">手续费由卖家承担；报价 24 小时内有效，只能成交一次。</p>
      <Button className="w-full" disabled={!valid} onClick={() => setOpen(true)}>
        <PenLine size={18} aria-hidden />
        签名报价
      </Button>
      {valid ? (
        <SignConfirmDialog
          open={open}
          onOpenChange={setOpen}
          action="签署担保报价"
          amount={amount.toString()}
          to={chain.contracts.Escrow}
          toLabel="担保合约"
          network={NETWORK_LABEL}
          rows={[
            { label: "买家", value: buyer ? shortAddress(buyer) : "任何人" },
            { label: "发货期 / 确认期", value: `${ship} 天 / ${confirm} 天` },
          ]}
          note="签名不会动用你的钱，只是授权买家按这些条款付款建单。"
          onConfirm={async () => {
            const sdk = await loadWalletSdk();
            const signed = await wallet.signOffer({
              title: title.trim(),
              amount,
              buyer: isAddress(buyer) ? buyer : undefined,
              shipDays: ship,
              confirmDays: confirm,
              validHours: 24,
            });
            if (!(await sdk.verifyOffer(signed))) throw new Error("签名没有通过链上验证，请重试");
            setOpen(false);
            setLink(`${location.origin}/pay?offer=${encodeSignedOffer(signed)}`);
          }}
        />
      ) : null}
    </Card>
  );
}

export function OfferSection() {
  return <WalletGate>{(wallet) => <OfferForm wallet={wallet} />}</WalletGate>;
}
