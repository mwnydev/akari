"use client";

import { ApiError, createApi, errorMessage } from "@akari/api-client";
import { Button, Field, Input } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { QueryClient, QueryClientProvider, useMutation } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";

const api = createApi();

const EmailSchema = z.object({
  email: z.email({ error: "请输入正确的邮箱地址" }),
});
const CodeSchema = z.object({
  code: z.string().check(z.regex(/^\d{6}$/, { error: "请输入 6 位数字验证码" })),
});
type EmailInput = z.infer<typeof EmailSchema>;
type CodeInput = z.infer<typeof CodeSchema>;

async function startLogin(email: string, captcha: string | null) {
  const { data, error, response } = await api.POST("/api/v1/auth/email/start", {
    body: { email, captcha },
  });
  if (!data) throw ApiError.from(error, response.status);
  return data;
}

async function verifyLogin(email: string, code: string) {
  const { data, error, response } = await api.POST("/api/v1/auth/email/verify", {
    body: { email, code },
  });
  if (!data) throw ApiError.from(error, response.status);
  return data;
}

/** 每秒减 1 的倒计时；`start(n)` 重新开始。 */
function useCountdown() {
  const [left, setLeft] = useState(0);
  useEffect(() => {
    if (left <= 0) return;
    const t = setTimeout(() => setLeft((n) => n - 1), 1000);
    return () => clearTimeout(t);
  }, [left]);
  return [left, setLeft] as const;
}

/**
 * Cloudflare Turnstile 占位：配置 NEXT_PUBLIC_TURNSTILE_SITE_KEY 后接入真实组件（P0 上线前）。
 * 后端未配置 TURNSTILE_SECRET 时不校验，开发环境传 null 即可。
 */
function useCaptcha(): string | null {
  return null;
}

function Steps({ next }: { next: string }) {
  const router = useRouter();
  const captcha = useCaptcha();
  const [email, setEmail] = useState<string | null>(null);
  const [resendIn, setResendIn] = useCountdown();

  const emailForm = useForm<EmailInput>({
    resolver: standardSchemaResolver(EmailSchema),
    defaultValues: { email: "" },
  });
  const codeForm = useForm<CodeInput>({
    resolver: standardSchemaResolver(CodeSchema),
    defaultValues: { code: "" },
  });

  const start = useMutation({
    mutationFn: (v: EmailInput) => startLogin(v.email.trim(), captcha),
    onSuccess: (res, v) => {
      setEmail(v.email.trim());
      setResendIn(res.resend_in);
      codeForm.reset();
    },
    onError: (e) => {
      if (e instanceof ApiError && e.code === "RESEND_TOO_SOON" && email) setResendIn(60);
    },
  });

  const verify = useMutation({
    mutationFn: (v: CodeInput) => verifyLogin(email ?? "", v.code),
    onSuccess: () => {
      router.replace(next);
      router.refresh();
    },
    onError: (e) => {
      if (e instanceof ApiError && e.code === "TOO_MANY_ATTEMPTS") codeForm.reset();
    },
  });

  if (!email) {
    const err = emailForm.formState.errors.email?.message;
    return (
      <form
        className="mt-6 flex flex-col gap-4"
        noValidate
        onSubmit={emailForm.handleSubmit((v) => start.mutate(v))}
      >
        <Field label="邮箱" error={err}>
          <Input
            id="login-email"
            type="email"
            autoComplete="email"
            inputMode="email"
            placeholder="you@example.com"
            aria-invalid={err ? true : undefined}
            {...emailForm.register("email")}
          />
        </Field>
        <div className="rounded-lg border border-dashed border-border p-3 text-sm text-fg-muted">
          人机验证（开发环境已跳过）
        </div>
        {start.isError ? (
          <p role="alert" className="text-sm text-danger">
            {errorMessage(start.error)}
          </p>
        ) : null}
        <Button type="submit" size="lg" disabled={start.isPending}>
          {start.isPending ? "发送中…" : "获取验证码"}
        </Button>
      </form>
    );
  }

  const err = codeForm.formState.errors.code?.message;
  return (
    <form
      className="mt-6 flex flex-col gap-4"
      noValidate
      onSubmit={codeForm.handleSubmit((v) => verify.mutate(v))}
    >
      <p className="text-sm text-fg-muted">
        验证码已发送到 <span className="font-medium text-fg">{email}</span>，10 分钟内有效。
      </p>
      <Field label="验证码" error={err}>
        <Input
          id="login-code"
          autoComplete="one-time-code"
          inputMode="numeric"
          maxLength={6}
          placeholder="6 位数字"
          className="amount tracking-[0.4em]"
          aria-invalid={err ? true : undefined}
          autoFocus
          {...codeForm.register("code")}
        />
      </Field>
      {verify.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(verify.error)}
        </p>
      ) : null}
      {start.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(start.error)}
        </p>
      ) : null}
      <Button type="submit" size="lg" disabled={verify.isPending}>
        {verify.isPending ? "登录中…" : "登录"}
      </Button>
      <div className="flex items-center justify-between text-sm">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            setEmail(null);
            start.reset();
            verify.reset();
          }}
        >
          换个邮箱
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={resendIn > 0 || start.isPending}
          onClick={() => start.mutate({ email })}
        >
          {resendIn > 0 ? `${resendIn} 秒后可重发` : "重新发送"}
        </Button>
      </div>
    </form>
  );
}

export function LoginForm({ next }: { next: string }) {
  const [client] = useState(
    () => new QueryClient({ defaultOptions: { mutations: { retry: false } } }),
  );
  return (
    <QueryClientProvider client={client}>
      <Steps next={next} />
    </QueryClientProvider>
  );
}
