"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Textarea } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import type { ReactNode } from "react";
import type { FieldErrors, UseFormRegister, UseFormReturn } from "react-hook-form";
import { PRESET_REASONS, type ReasonValues } from "./reason";

export const selectClass = "h-10 w-full rounded-md border border-border bg-surface px-3 text-fg";

/** 理由两项（预设 + 补充说明），放进各个管理弹窗自己的表单里（表单值须含 `preset`、`note`）。 */
export function ReasonFields<T extends ReasonValues>({ form }: { form: UseFormReturn<T> }) {
  // 只用到 ReasonValues 的两个字段；泛型表单的 register 对具体字段名收窄不了，这里按子集看待
  const register = form.register as unknown as UseFormRegister<ReasonValues>;
  const errors = form.formState.errors as FieldErrors<ReasonValues>;
  return (
    <>
      <Field label="理由" error={errors.preset?.message}>
        <select className={selectClass} {...register("preset")}>
          {PRESET_REASONS.map((r) => (
            <option key={r}>{r}</option>
          ))}
        </select>
      </Field>
      <Field label="补充说明（选「其他」时必填）" error={errors.note?.message}>
        <Textarea maxLength={400} className="h-20 resize-none" {...register("note")} />
      </Field>
    </>
  );
}

/**
 * 管理操作弹窗的外壳：标题、表单、服务端错误（含 403：界面按 /me 显示入口，但权限以服务端为准，
 * 例如刚被免职或权限被自动暂停）与确认按钮。
 */
export function ModDialog({
  title,
  description = "理由会写进管理日志并通知对方，操作可由站长撤销",
  confirmLabel,
  danger = false,
  pending,
  error,
  onSubmit,
  onClose,
  children,
}: {
  title: string;
  description?: string;
  confirmLabel: string;
  danger?: boolean;
  pending: boolean;
  error: unknown;
  onSubmit: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent title={title} description={description}>
        <form
          className="space-y-3"
          noValidate
          onSubmit={(e) => {
            e.preventDefault();
            onSubmit();
          }}
        >
          {children}
          {error ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2 pt-1">
            <Button variant="secondary" onClick={onClose}>
              取消
            </Button>
            <Button type="submit" variant={danger ? "danger" : "primary"} disabled={pending}>
              {pending ? "提交中…" : confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
