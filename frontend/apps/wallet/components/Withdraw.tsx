"use client";

import { DEFAULT_PARAMS } from "@akari/fees";
import { Amount, Badge, Button, Field, Input, formatUsdt, shortAddress } from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { toast } from "@akari/ui/toast";
import { useState } from "react";
import { wallet } from "../lib/mock";

/** 解析用户输入的 USDT 金额为最小单位；只用字符串与 bigint，不经过浮点。 */
function parseUsdt(input: string): bigint | null {
  const m = /^(\d{1,12})(?:\.(\d{0,6}))?$/.exec(input.trim());
  if (!m) return null;
  return BigInt(m[1] ?? "0") * 1_000_000n + BigInt((m[2] ?? "").padEnd(6, "0"));
}

export function Withdraw() {
  const [to, setTo] = useState(0);
  const [raw, setRaw] = useState("");
  const [open, setOpen] = useState(false);
  const balance = BigInt(wallet.balance);
  const fee = DEFAULT_PARAMS.sponsorFee;
  const amount = parseUsdt(raw);
  const valid = amount !== null && amount > fee && amount <= balance;
  const target = wallet.savedAddresses[to];

  return (
    <div className="space-y-3">
      <fieldset>
        <legend className="mb-1 text-sm font-medium">提到</legend>
        <div className="space-y-2">
          {wallet.savedAddresses.map((a, i) => (
            <label
              key={a.address}
              className="flex cursor-pointer items-center gap-3 rounded-lg border border-border p-3 has-checked:border-accent has-disabled:cursor-not-allowed has-disabled:opacity-60"
            >
              <input
                type="radio"
                name="to"
                checked={to === i}
                disabled={a.activeAt !== null}
                onChange={() => setTo(i)}
                className="accent-(--color-accent)"
              />
              <span className="min-w-0 flex-1">
                <span className="block font-medium">{a.label}</span>
                <span className="amount block font-mono text-sm text-fg-muted">
                  {shortAddress(a.address)}
                </span>
              </span>
              {a.activeAt ? <Badge tone="warning">{a.activeAt}</Badge> : null}
            </label>
          ))}
        </div>
      </fieldset>
      <Field label="金额" hint={`代付费 ${formatUsdt(fee)} USDT 从提出金额中扣除`}>
        <div className="flex gap-2">
          <Input
            inputMode="decimal"
            placeholder="0.00"
            value={raw}
            onChange={(e) => setRaw(e.target.value)}
            className="amount"
          />
          <Button
            variant="secondary"
            onClick={() => setRaw(formatUsdt(balance).replaceAll(",", ""))}
          >
            全部
          </Button>
        </div>
      </Field>
      {amount !== null ? (
        <p className="text-sm text-fg-muted">
          交易所到账 <Amount value={amount > fee ? amount - fee : 0n} />
        </p>
      ) : null}
      <Button className="w-full" disabled={!valid || !target} onClick={() => setOpen(true)}>
        提回交易所
      </Button>
      {target && amount !== null ? (
        <SignConfirmDialog
          open={open}
          onOpenChange={setOpen}
          action="提回交易所"
          amount={amount.toString()}
          to={target.address}
          toLabel={`${target.label} 充值地址`}
          rows={[
            { label: "代付费", value: <Amount value={fee} /> },
            { label: "到账", value: <Amount value={amount - fee} /> },
          ]}
          note="请确认交易所充值页选择的也是 Arbitrum One 网络。"
          onConfirm={async () => {
            await new Promise((r) => setTimeout(r, 900));
            setOpen(false);
            toast.success("已提交，约 1 分钟内上链");
          }}
        />
      ) : null}
    </div>
  );
}
