"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Input } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { KeyRound, Mail } from "lucide-react";
import { useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { enrollRulingPasskey, sendEnrollCode } from "../../lib/ruling-passkey";

const Schema = z.object({
  code: z.string().check(z.regex(/^\d{6}$/, { error: "请输入邮件里的 6 位数字" })),
});

/**
 * 登记第一个裁决 Passkey：向账号邮箱发一封写明用途的验证码 → 创建 Passkey（RP ID 是论坛主站）。
 * 它只用于签署裁决，与钱包 Passkey、后台 Passkey 互不相通。
 */
export default function RulingPasskeySetup({ onDone }: { onDone: () => void }) {
  const [sent, setSent] = useState(false);
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { code: "" },
  });
  const send = useMutation({ mutationFn: sendEnrollCode, onSuccess: () => setSent(true) });
  const enroll = useMutation({
    mutationFn: ({ code }: { code: string }) => enrollRulingPasskey(code),
    onSuccess: onDone,
  });
  return (
    <div
      className="space-y-3 rounded-md border border-border p-3"
      data-testid="ruling-passkey-setup"
    >
      <div>
        <p className="font-medium">登记裁决 Passkey</p>
        <p className="text-sm text-fg-muted">
          裁决决定托管资金的分配，提交时要用裁决 Passkey 确认。第一次使用先做一次邮箱验证。
        </p>
      </div>
      {!sent ? (
        <Button onClick={() => send.mutate()} disabled={send.isPending}>
          <Mail size={16} aria-hidden />
          {send.isPending ? "发送中…" : "发送验证码到我的邮箱"}
        </Button>
      ) : (
        <form
          className="flex flex-wrap items-end gap-2"
          noValidate
          onSubmit={form.handleSubmit((v) => enroll.mutate(v))}
        >
          <div className="w-40">
            <Field label="邮箱验证码" error={form.formState.errors.code?.message}>
              <Input inputMode="numeric" autoComplete="one-time-code" {...form.register("code")} />
            </Field>
          </div>
          <Button type="submit" disabled={enroll.isPending}>
            <KeyRound size={16} aria-hidden />
            {enroll.isPending ? "等待 Passkey…" : "验证并创建"}
          </Button>
        </form>
      )}
      {send.isError || enroll.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(send.error ?? enroll.error)}
        </p>
      ) : null}
    </div>
  );
}
