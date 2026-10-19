"use client";

import { ApiError, errorMessage } from "@akari/api-client";
import { Button, Card, Field, Input } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { KeyRound, Mail } from "lucide-react";
import { useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap, type AdminSessionView } from "../../lib/api";
import { FORUM_ORIGIN } from "../../lib/config";
import { loginWithPasskey, registerPasskey } from "../../lib/passkey";

const EXPIRED: Record<"idle" | "absolute" | "server", string> = {
  idle: "闲置超过 2 小时，后台会话已过期，请重新验证。",
  absolute: "后台会话最长 12 小时，已到期，请重新验证。",
  server: "后台会话已失效（过期、在别处退出或权限变更），请重新验证。",
};

const CodeSchema = z.object({
  code: z.string().check(z.regex(/^\d{6}$/, { error: "请输入邮件里的 6 位数字" })),
});

type Step = "login" | "enroll";

/**
 * 后台登录（ADR 0013 第 4 条）：先在论坛用邮箱登录站长账号，再在这里用后台 Passkey 登录。
 * 还没有后台 Passkey 时走一次专用的邮箱验证码，绑定第一个 Passkey（绑定即登录）。
 */
export function LoginGate({
  expired,
  onLogin,
}: {
  expired: "idle" | "absolute" | "server" | null;
  onLogin: (view: AdminSessionView) => void;
}) {
  const [step, setStep] = useState<Step>("login");
  const [blocked, setBlocked] = useState<"forum" | "owner" | null>(null);

  const login = useMutation({
    mutationFn: loginWithPasskey,
    onSuccess: onLogin,
    onError: (e) => {
      if (!(e instanceof ApiError)) return;
      if (e.code === "UNAUTHORIZED") setBlocked("forum");
      else if (e.code === "FORBIDDEN") setBlocked("owner");
      else if (e.code === "REVERIFY_REQUIRED") setStep("enroll");
    },
  });

  return (
    <main className="mx-auto max-w-md space-y-4 px-4 py-16">
      <div>
        <h1 className="text-2xl font-semibold">Akari 后台</h1>
        <p className="text-sm text-fg-muted">只有站长能进入，需要后台 Passkey。</p>
      </div>
      {expired ? (
        <p role="status" className="rounded-md bg-warning-soft px-3 py-2 text-sm text-warning">
          {EXPIRED[expired]}
        </p>
      ) : null}
      {blocked === "forum" ? (
        <Card className="space-y-2">
          <p className="font-medium">请先在论坛登录站长账号</p>
          <p className="text-sm text-fg-muted">
            后台登录要求论坛里已用邮箱登录的站长账号，再用后台 Passkey 确认。登录后回到本页重试。
          </p>
          <Button asChild variant="secondary">
            <a href={`${FORUM_ORIGIN}/login`}>去论坛登录</a>
          </Button>
        </Card>
      ) : blocked === "owner" ? (
        <Card>
          <p role="alert" className="text-sm text-danger">
            当前论坛账号不是站长，不能进入后台。管理员与版主在论坛里直接管理。
          </p>
        </Card>
      ) : null}
      {step === "login" ? (
        <Card className="space-y-3">
          <Button onClick={() => login.mutate()} disabled={login.isPending} className="w-full">
            <KeyRound size={16} aria-hidden />
            {login.isPending ? "等待 Passkey…" : "用后台 Passkey 登录"}
          </Button>
          {login.isError &&
          !(
            login.error instanceof ApiError &&
            ["UNAUTHORIZED", "FORBIDDEN"].includes(login.error.code)
          ) ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(login.error)}
            </p>
          ) : null}
        </Card>
      ) : (
        <Enroll onLogin={onLogin} onBack={() => setStep("login")} />
      )}
    </main>
  );
}

/** 绑定第一个后台 Passkey：专用邮箱验证码（10 分钟内有效、一次性）→ 创建 Passkey。 */
function Enroll({
  onLogin,
  onBack,
}: {
  onLogin: (view: AdminSessionView) => void;
  onBack: () => void;
}) {
  const [sent, setSent] = useState(false);
  const form = useForm<z.infer<typeof CodeSchema>>({
    resolver: standardSchemaResolver(CodeSchema),
    defaultValues: { code: "" },
  });
  const send = useMutation({
    mutationFn: () => unwrap(api.POST("/api/v1/admin/enroll/code")),
    onSuccess: () => setSent(true),
  });
  const enroll = useMutation({
    mutationFn: async ({ code }: { code: string }) => {
      await unwrap(api.POST("/api/v1/admin/enroll/verify", { body: { code } }));
      await registerPasskey();
      return unwrap(api.GET("/api/v1/admin/session"));
    },
    onSuccess: onLogin,
  });
  const alreadyEnrolled = send.error instanceof ApiError && send.error.code === "CONFLICT";

  return (
    <Card className="space-y-3">
      <div>
        <p className="font-medium">首次使用：绑定后台 Passkey</p>
        <p className="text-sm text-fg-muted">
          向站长账号邮箱发一封专用验证码，验证后创建后台 Passkey（只属于后台域名，与钱包、论坛的
          Passkey 分开）。 每次绑定都会推送管理员群。
        </p>
      </div>
      {!sent ? (
        <Button onClick={() => send.mutate()} disabled={send.isPending} className="w-full">
          <Mail size={16} aria-hidden />
          {send.isPending ? "发送中…" : "发送验证码到站长邮箱"}
        </Button>
      ) : (
        <form
          className="space-y-3"
          noValidate
          onSubmit={form.handleSubmit((v) => enroll.mutate(v))}
        >
          <Field label="邮箱验证码" error={form.formState.errors.code?.message}>
            <Input inputMode="numeric" autoComplete="one-time-code" {...form.register("code")} />
          </Field>
          <Button type="submit" disabled={enroll.isPending} className="w-full">
            <KeyRound size={16} aria-hidden />
            {enroll.isPending ? "等待 Passkey…" : "验证并创建后台 Passkey"}
          </Button>
        </form>
      )}
      {alreadyEnrolled ? (
        <p role="alert" className="text-sm text-danger">
          这个账号已经绑定过后台 Passkey，请直接登录；丢失全部 Passkey 时按站长账号找回流程处理。
        </p>
      ) : send.isError || enroll.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(send.error ?? enroll.error)}
        </p>
      ) : null}
      <Button variant="ghost" size="sm" onClick={onBack}>
        返回登录
      </Button>
    </Card>
  );
}
