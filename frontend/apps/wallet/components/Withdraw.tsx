"use client";

import { Amount, Button, Field, Input, formatUsdt, parseUsdt, shortAddress } from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { CircleCheck } from "lucide-react";
import { useState } from "react";
import type { WithdrawAddressView } from "../lib/wallet/api";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import type { Wallet } from "../lib/wallet/sdk";
import { sponsorFeeFor } from "../lib/wallet/sponsor-fee";
import { TxLink } from "./TxLink";
import {
  addressUsable,
  canSponsorWithdraw,
  checkAddress,
  maxWithdraw,
  withdrawError,
} from "../lib/wallet/withdraw";

interface Done {
  hash: string;
  amount: bigint;
  label: string;
}

/** 去掉千分位，填回输入框。 */
const plain = (v: bigint) => formatUsdt(v).replaceAll(",", "");

/**
 * 提回交易所：只能提到已生效的提币地址（forum-sponsor 只代付这种转账）。代付费从钱包另扣，
 * 「全部提回」= 余额 − 代付费；确认页明文显示金额、地址前后 6 位、网络与代付费，再 Face ID。
 */
export function Withdraw({
  wallet,
  balance,
  credits,
  addresses,
  now,
  onWithdrawn,
}: {
  wallet: Wallet;
  /** USDT 余额；读取中为 null */
  balance: bigint | null;
  /** 免费代付提币：剩余次数、今日还可几次；读取中为 null */
  credits: { left: number; today: number } | null;
  addresses: WithdrawAddressView[];
  now: number;
  onWithdrawn: () => void;
}) {
  const usable = addresses.filter((a) => addressUsable(a, now));
  const [toId, setToId] = useState<number | null>(null);
  const [raw, setRaw] = useState("");
  const [open, setOpen] = useState(false);
  const [done, setDone] = useState<Done | null>(null);
  const fee = sponsorFeeFor("transfer");
  const target = usable.find((a) => a.id === toId) ?? usable[0];
  // 后端给的地址也再校验一遍：不能是自己或本站合约
  const dest =
    target &&
    checkAddress(target.address, {
      self: wallet.address,
      contracts: Object.values(chain.contracts),
    });
  const amount = parseUsdt(raw);
  const amountError =
    raw.trim() === "" || balance === null ? null : withdrawError(amount, balance, fee);
  const error = dest && !dest.ok ? dest.error : amountError;
  const sponsored = credits !== null && canSponsorWithdraw(credits.left, credits.today);
  const ready = sponsored && balance !== null && amount !== null && amount > 0n && error === null;

  if (!target) {
    return (
      <p className="text-sm text-fg-muted">
        {addresses.some((a) => a.status !== "revoked")
          ? "新地址还在 24 小时冷却期，生效后就能提回。"
          : `先在下方「提币地址」里保存交易所的 ${NETWORK_LABEL} 充值地址，再来提回。`}
      </p>
    );
  }
  return (
    <div className="space-y-3">
      <fieldset>
        <legend className="mb-1 text-sm font-medium">提到</legend>
        <div className="space-y-2">
          {usable.map((a) => (
            <label
              key={a.id}
              className="flex cursor-pointer items-center gap-3 rounded-lg border border-border p-3 has-checked:border-accent"
            >
              <input
                type="radio"
                name="withdraw-to"
                checked={target.id === a.id}
                onChange={() => setToId(a.id)}
                className="accent-(--color-accent)"
              />
              <span className="min-w-0 flex-1">
                <span className="block font-medium">{a.label}</span>
                <span className="amount block font-mono text-sm text-fg-muted">
                  {shortAddress(a.address)}
                </span>
              </span>
            </label>
          ))}
        </div>
      </fieldset>
      <Field label="到账金额（USDT）" hint={`代付费 ${formatUsdt(fee)} USDT 另从钱包扣除`}>
        <div className="flex gap-2">
          <Input
            inputMode="decimal"
            placeholder="0.00"
            value={raw}
            onChange={(e) => {
              setRaw(e.target.value);
              setDone(null);
            }}
            className="amount"
            aria-invalid={error !== null}
          />
          <Button
            variant="secondary"
            disabled={balance === null || maxWithdraw(balance, fee) === 0n}
            onClick={() => {
              if (balance !== null) setRaw(plain(maxWithdraw(balance, fee)));
              setDone(null);
            }}
          >
            全部提回
          </Button>
        </div>
      </Field>
      {credits ? (
        <p className="text-sm text-fg-muted" data-testid="withdraw-credits">
          免费代付提币剩余 {credits.left} 次（今日还可 {credits.today} 次）
        </p>
      ) : null}
      {credits && !sponsored ? (
        <p role="alert" className="rounded-md bg-warning-soft p-3 text-sm text-warning">
          {/* 与 NO_WITHDRAW_CREDIT 同一句话，「紧急取回」做成链接 */}
          完成一笔担保交易后可再获得 1 次；也可在
          <a href="/emergency" className="underline">
            「紧急取回」
          </a>
          页自付 ETH 提取
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      ) : ready ? (
        <p className="text-sm text-fg-muted">
          钱包共扣除 <Amount value={amount + fee} />
          （含代付费）
        </p>
      ) : null}
      <Button className="w-full" disabled={!ready} onClick={() => setOpen(true)}>
        提回交易所
      </Button>
      {done ? (
        <p
          role="status"
          className="flex flex-wrap items-center gap-1 text-sm text-success"
          data-testid="withdraw-done"
        >
          <CircleCheck size={16} aria-hidden />
          已提回 <Amount value={done.amount} /> 到{done.label}，交易 <TxLink hash={done.hash} />
        </p>
      ) : null}
      {ready && dest?.ok ? (
        <SignConfirmDialog
          open={open}
          onOpenChange={setOpen}
          action="提回交易所"
          amount={(amount + fee).toString()}
          to={dest.address}
          toLabel={`${target.label} 充值地址`}
          network={NETWORK_LABEL}
          rows={[
            { label: "交易所到账", value: <Amount value={amount} /> },
            { label: "代付费", value: <Amount value={fee} /> },
          ]}
          note={`请确认交易所充值页选择的也是 ${NETWORK_LABEL} 网络，转出后无法撤回。`}
          onConfirm={async () => {
            const hash = await wallet.withdraw(dest.address, amount);
            setOpen(false);
            setRaw("");
            setDone({ hash, amount, label: target.label });
            onWithdrawn();
          }}
        />
      ) : null}
    </div>
  );
}
