import type { ComponentProps, ReactNode } from "react";
import { cn } from "../cn";

const base =
  "w-full rounded-md border border-border bg-surface px-3 text-fg placeholder:text-fg-muted " +
  "focus-visible:outline-2 focus-visible:outline-offset-0 focus-visible:outline-accent " +
  "disabled:opacity-50";

// React 19 起 ref 是普通 prop：ComponentProps 带 ref，react-hook-form 的 register() 可直接展开。
export function Input({ className, ...props }: ComponentProps<"input">) {
  return <input className={cn(base, "h-10", className)} {...props} />;
}

export function Textarea({ className, ...props }: ComponentProps<"textarea">) {
  return <textarea className={cn(base, "min-h-24 py-2", className)} {...props} />;
}

/** 表单字段：标签 + 控件 + 提示或错误（错误优先，读屏器会朗读）。 */
export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string;
  hint?: string;
  error?: string | undefined;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-medium">{label}</span>
      {children}
      {error ? (
        <span role="alert" className="mt-1 block text-sm text-danger">
          {error}
        </span>
      ) : hint ? (
        <span className="mt-1 block text-sm text-fg-muted">{hint}</span>
      ) : null}
    </label>
  );
}
