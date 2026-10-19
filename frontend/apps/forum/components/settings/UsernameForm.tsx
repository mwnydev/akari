"use client";

import { ApiError, errorMessage, type ErrorCode } from "@akari/api-client";
import { Button, Field, Input } from "@akari/ui";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { profileHref } from "../../lib/display";
import { usernameProblem } from "../../lib/username";

/** 这些错误针对输入的名字本身，显示在输入框下面；其余（网络、限频……）显示在按钮旁。 */
const FIELD_ERRORS: ReadonlySet<ErrorCode | "NETWORK"> = new Set([
  "INVALID_USERNAME",
  "USERNAME_TAKEN",
  "USERNAME_CHANGE_TOO_SOON",
]);

/** 设置 / 修改用户名。保存后刷新服务端组件，导航栏与个人主页立即换成新名字。 */
export function UsernameForm({ id, current }: { id: number; current: string | null }) {
  const router = useRouter();
  const [value, setValue] = useState(current ?? "");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [refreshing, startTransition] = useTransition();

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSaved(null);
    setFormError(null);
    const problem = usernameProblem(value);
    setFieldError(problem);
    if (problem) return;
    if (value.trim() === current) return setFieldError("和现在的用户名一样");
    setPending(true);
    try {
      const res = await unwrap(
        api.PUT("/api/v1/me/username", { body: { username: value.trim() } }),
      );
      setValue(res.username);
      setSaved(res.username);
      startTransition(() => router.refresh());
    } catch (err) {
      const msg = errorMessage(err);
      if (err instanceof ApiError && FIELD_ERRORS.has(err.code)) setFieldError(msg);
      else setFormError(msg);
    } finally {
      setPending(false);
    }
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-3" aria-label="用户名">
      <Field
        label="用户名"
        hint="2–20 个字：汉字、字母、数字、下划线，不能是纯数字。设置后 30 天内不能再改。"
        error={fieldError ?? undefined}
      >
        <Input
          value={value}
          maxLength={40}
          autoComplete="username"
          aria-invalid={fieldError !== null}
          onChange={(e) => {
            setValue(e.target.value);
            setFieldError(null);
          }}
        />
      </Field>
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" disabled={pending || refreshing}>
          {pending ? "保存中…" : "保存"}
        </Button>
        {saved ? (
          <span role="status" className="text-sm text-success">
            已保存。
            <Link href={profileHref({ id, username: saved })} className="text-accent underline">
              查看个人主页
            </Link>
          </span>
        ) : null}
        {formError ? (
          <span role="alert" className="text-sm text-danger">
            {formError}
          </span>
        ) : null}
      </div>
    </form>
  );
}
