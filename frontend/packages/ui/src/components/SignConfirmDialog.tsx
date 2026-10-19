"use client";

import { LoaderCircle, ScanFace, ShieldCheck } from "lucide-react";
import { useState, type ReactNode } from "react";
import { shortAddress } from "../address";
import { Amount } from "../Amount";
import { Button } from "./Button";
import { Dialog, DialogContent } from "./Dialog";
import { NetworkBadge } from "./NetworkBadge";

export interface ConfirmRow {
  label: string;
  value: ReactNode;
}

export interface SignConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 动作，如「担保付款」「提回交易所」 */
  action: string;
  /** 本次从钱包转出的总额（最小单位） */
  amount: string;
  /** 收款方：合约或交易所地址 */
  to: string;
  toLabel: string;
  rows?: ConfirmRow[];
  note?: ReactNode;
  /** 调起 Passkey 签名；抛错时显示人话错误 */
  onConfirm: () => Promise<void>;
}

/**
 * 签名确认弹窗：动钱前先看清动作、金额、网络、地址前后 6 位，再调 Face ID（plan 第 11 节）。
 * 全站动钱只走这一个组件。
 */
export function SignConfirmDialog({
  open,
  onOpenChange,
  action,
  amount,
  to,
  toLabel,
  rows = [],
  note,
  onConfirm,
}: SignConfirmDialogProps) {
  const [phase, setPhase] = useState<"idle" | "signing" | "error">("idle");
  const [error, setError] = useState("");

  async function confirm() {
    setPhase("signing");
    try {
      await onConfirm();
      setPhase("idle");
    } catch (e) {
      setError(e instanceof Error ? e.message : "签名没有完成，请重试");
      setPhase("error");
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (phase !== "signing") onOpenChange(o);
      }}
    >
      <DialogContent title={`确认${action}`} description="请核对以下信息，确认无误后再验证 Face ID">
        <div className="rounded-lg bg-surface p-4">
          <p className="text-sm text-fg-muted">{action}</p>
          <p className="mt-0.5 text-3xl font-semibold">
            <Amount value={amount} />
          </p>
          <dl className="mt-4 space-y-2 text-sm">
            <div className="flex items-center justify-between gap-3">
              <dt className="text-fg-muted">网络</dt>
              <dd>
                <NetworkBadge />
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="text-fg-muted">{toLabel}</dt>
              <dd className="amount font-mono" title={to}>
                {shortAddress(to)}
              </dd>
            </div>
            {rows.map((r) => (
              <div key={r.label} className="flex items-center justify-between gap-3">
                <dt className="text-fg-muted">{r.label}</dt>
                <dd className="text-right">{r.value}</dd>
              </div>
            ))}
          </dl>
        </div>
        {note ? <div className="mt-3 text-sm text-fg-muted">{note}</div> : null}
        {phase === "error" ? (
          <p role="alert" className="mt-3 rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
            {error}
          </p>
        ) : null}
        <Button size="lg" className="mt-4 w-full" disabled={phase === "signing"} onClick={confirm}>
          {phase === "signing" ? (
            <LoaderCircle size={20} className="animate-spin" aria-hidden />
          ) : (
            <ScanFace size={20} aria-hidden />
          )}
          {phase === "signing" ? "等待 Face ID…" : "使用 Face ID / 指纹确认"}
        </Button>
        <p className="mt-3 flex items-center justify-center gap-1 text-xs text-fg-muted">
          <ShieldCheck size={14} aria-hidden />
          gas 由平台代付，钱包无需 ETH
        </p>
      </DialogContent>
    </Dialog>
  );
}
