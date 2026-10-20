"use client";

import { ApiError } from "@akari/api-client";
import {
  Amount,
  Button,
  Card,
  CardTitle,
  EmptyState,
  ErrorState,
  Field,
  Input,
  LoadingState,
  NetworkBadge,
  cn,
  useCountdown,
} from "@akari/ui";
import { useMutation } from "@tanstack/react-query";
import { KeyRound, LoaderCircle, MailCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { userMessage, walletApi, type WalletOverview } from "../lib/wallet/api";
import { NETWORK_LABEL } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import type { Wallet } from "../lib/wallet/sdk";
import { useWallet } from "../lib/wallet/useWallet";
import { LoginPrompt, useForumHref } from "./WalletGate";

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

function ErrorLine({ error }: { error: string }) {
  return error ? (
    <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
      {error}
    </p>
  ) : null;
}

const STEPS = ["邮箱验证", "创建 Passkey", "登记钱包地址"];

function Steps({ current }: { current: number }) {
  return (
    <ol className="flex flex-wrap gap-x-4 gap-y-1 text-sm" aria-label="开通步骤">
      {STEPS.map((s, i) => (
        <li
          key={s}
          aria-current={i === current ? "step" : undefined}
          className={cn(i === current ? "font-medium text-accent" : "text-fg-muted")}
        >
          {i + 1}. {s}
        </li>
      ))}
    </ol>
  );
}

/** 第 1 步：开通交易专用的邮箱验证码（与登录码分开，10 分钟内注册 Passkey 有效）。 */
function EmailStep({ onVerified }: { onVerified: () => void }) {
  const api = walletApi();
  const [code, setCode] = useState("");
  const [resendIn, setResendIn] = useCountdown();
  const send = useMutation({
    mutationFn: () => api.startReverify(),
    onSuccess: (r) => setResendIn(r.resend_in),
  });
  const verify = useMutation({ mutationFn: () => api.verifyReverify(code), onSuccess: onVerified });
  const error = send.isError
    ? userMessage(send.error, "验证码发送失败，请稍后再试")
    : verify.isError
      ? userMessage(verify.error, "验证没有完成，请重试")
      : "";

  if (!send.isSuccess) {
    return (
      <>
        <p className="text-sm text-fg-muted">
          为了确认是你本人在开通交易，先向账号邮箱发一个验证码（与登录验证码不同）。
        </p>
        <ErrorLine error={error} />
        <Button
          size="lg"
          className="w-full"
          disabled={send.isPending}
          onClick={() => send.mutate()}
        >
          <MailCheck size={20} aria-hidden />
          {send.isPending ? "发送中…" : "发送验证码到邮箱"}
        </Button>
      </>
    );
  }
  return (
    <form
      className="space-y-3"
      noValidate
      onSubmit={(e) => {
        e.preventDefault();
        verify.mutate();
      }}
    >
      <p className="text-sm text-fg-muted">验证码已发到账号邮箱，10 分钟内有效。</p>
      <Field label="邮箱验证码">
        <Input
          name="code"
          autoComplete="one-time-code"
          inputMode="numeric"
          maxLength={6}
          placeholder="6 位数字"
          className="amount tracking-[0.4em]"
          value={code}
          onChange={(e) => setCode(e.target.value.trim())}
        />
      </Field>
      <ErrorLine error={error} />
      <Button
        type="submit"
        size="lg"
        className="w-full"
        disabled={!/^\d{6}$/.test(code) || verify.isPending}
      >
        {verify.isPending ? "验证中…" : "验证"}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled={resendIn > 0 || send.isPending}
        onClick={() => send.mutate()}
      >
        {resendIn > 0 ? `${resendIn} 秒后可重发` : "重新发送"}
      </Button>
    </form>
  );
}

/** 第 2–3 步：创建 Passkey（一次 Face ID）→ 算出地址并登记；Passkey 已有时只补登记地址。 */
function PasskeyStep({
  overview,
  onReady,
  onReverifyExpired,
}: {
  overview: WalletOverview;
  onReady: (wallet: Wallet) => void;
  onReverifyExpired: () => void;
}) {
  const registered = overview.passkeys.length > 0;
  const create = useMutation({
    mutationFn: async () => {
      const sdk = await loadWalletSdk();
      return registered
        ? sdk.bindRegisteredPasskey(walletApi(), overview)
        : sdk.createWallet(walletApi());
    },
    onSuccess: onReady,
    onError: (e) => {
      if (e instanceof ApiError && e.code === "REVERIFY_REQUIRED") onReverifyExpired();
    },
  });
  return (
    <>
      <p className="text-sm text-fg-muted">
        {registered
          ? "钱包 Passkey 已创建，但钱包地址还没登记。用 Passkey 确认一次即可完成。"
          : "会在这台设备上创建一个「Akari 钱包签名（勿删）」Passkey，只用于确认付款和敏感操作。iCloud 钥匙串 / Google 密码管理器会自动同步，请不要删除这条记录。"}
      </p>
      <ErrorLine
        error={create.isError ? userMessage(create.error, "创建 Passkey 没有完成，请重试") : ""}
      />
      <Button
        size="lg"
        className="w-full"
        disabled={create.isPending}
        onClick={() => create.mutate()}
      >
        {create.isPending ? (
          <LoaderCircle size={20} className="animate-spin" aria-hidden />
        ) : (
          <KeyRound size={20} aria-hidden />
        )}
        {registered ? "用 Passkey 确认并登记地址" : "创建钱包 Passkey"}
      </Button>
    </>
  );
}

/** 开通交易前要先在论坛设置用户名（钱包 Passkey 以用户名命名，交易对手与仲裁员按用户名认人）。 */
function UsernameFirst() {
  const href = useForumHref("/settings");
  return (
    <Card className="space-y-3">
      <CardTitle>开通交易</CardTitle>
      <EmptyState
        title="先设置用户名"
        description="开通交易前要先在论坛设置用户名：交易对手和仲裁员都按用户名认人。设置后会回到这里继续。"
        action={
          <Button asChild>
            <a href={href}>去论坛设置用户名</a>
          </Button>
        }
      />
    </Card>
  );
}

function SetupFlow({
  overview,
  onReady,
}: {
  overview: WalletOverview;
  onReady: (wallet: Wallet) => void;
}) {
  // 第一个 Passkey 需要邮箱再验证；已注册过 Passkey（只差登记地址）直接到最后一步
  const [emailDone, setEmailDone] = useState(overview.passkeys.length > 0);
  const [expired, setExpired] = useState(false);
  if (overview.needs_username) return <UsernameFirst />;
  return (
    <Card className="space-y-3">
      <CardTitle>开通交易</CardTitle>
      <Steps current={!emailDone ? 0 : overview.passkeys.length > 0 ? 2 : 1} />
      {expired && !emailDone ? <ErrorLine error="邮箱验证已过期，请重新获取验证码" /> : null}
      {emailDone ? (
        <PasskeyStep
          overview={overview}
          onReady={onReady}
          onReverifyExpired={() => {
            setExpired(true);
            setEmailDone(false);
          }}
        />
      ) : (
        <EmailStep onVerified={() => setEmailDone(true)} />
      )}
    </Card>
  );
}

/** 开通交易（plan 第 5 节）：邮箱再验证 → 创建钱包 Passkey → 算出智能账户地址并登记。 */
export function WalletSetup() {
  const { state, setReady } = useWallet();
  switch (state.status) {
    case "loading":
      return <LoadingState rows={2} label="正在读取钱包" />;
    case "error":
      return <ErrorState message={state.message} />;
    case "signed-out":
      return <LoginPrompt />;
    case "setup":
      return <SetupFlow overview={state.overview} onReady={setReady} />;
    case "ready":
      return <WalletCard wallet={state.wallet} />;
  }
}
