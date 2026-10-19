"use client";

import { ApiError, errorMessage } from "@akari/api-client";
import { Button, Field, Input, useCountdown } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { QueryProvider } from "../../components/QueryProvider";
import { api, unwrap } from "../../lib/browser-api";
import { TURNSTILE_SITE_KEY } from "../../lib/public-config";
import { Turnstile } from "../../components/captcha/Turnstile";

const EmailSchema = z.object({
  email: z.email({ error: "请输入正确的邮箱地址" }),
});
const CodeSchema = z.object({
  code: z.string().check(z.regex(/^\d{6}$/, { error: "请输入 6 位数字验证码" })),
});
type EmailInput = z.infer<typeof EmailSchema>;
type CodeInput = z.infer<typeof CodeSchema>;

const startLogin = (email: string, captcha: string | null) =>
  unwrap(api.POST("/api/v1/auth/email/start", { body: { email, captcha } }));

const verifyLogin = (email: string, code: string) =>
  unwrap(api.POST("/api/v1/auth/email/verify", { body: { email, code } }));

function Steps({ next }: { next: string }) {
  const router = useRouter();
  // 配置了 NEXT_PUBLIC_TURNSTILE_SITE_KEY 才做人机验证；否则传 null（后端未配置 TURNSTILE_SECRET 时不校验）
  const [captcha, setCaptcha] = useState<string | null>(null);
  const [captchaRound, setCaptchaRound] = useState(0);
  const captchaMissing = TURNSTILE_SITE_KEY !== "" && captcha === null;
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
    // Turnstile 令牌只能用一次，无论成败都换新的
    onSettled: () => {
      setCaptcha(null);
      setCaptchaRound((n) => n + 1);
    },
  });

  const turnstile = TURNSTILE_SITE_KEY ? (
    <Turnstile
      siteKey={TURNSTILE_SITE_KEY}
      action="login"
      round={captchaRound}
      onToken={setCaptcha}
    />
  ) : null;

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
        {turnstile}
        {start.isError ? (
          <p role="alert" className="text-sm text-danger">
            {errorMessage(start.error)}
          </p>
        ) : null}
        <Button type="submit" size="lg" disabled={start.isPending || captchaMissing}>
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
      {turnstile}
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
          disabled={resendIn > 0 || start.isPending || captchaMissing}
          onClick={() => start.mutate({ email })}
        >
          {resendIn > 0 ? `${resendIn} 秒后可重发` : "重新发送"}
        </Button>
      </div>
    </form>
  );
}

export function LoginForm({ next }: { next: string }) {
  return (
    <QueryProvider>
      <Steps next={next} />
    </QueryProvider>
  );
}
