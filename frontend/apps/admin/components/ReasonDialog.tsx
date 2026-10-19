"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Textarea } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";

/** 与后端 `moderation::reason` 一致：去掉空白后 1–500 字。 */
const Schema = z.object({
  reason: z.string().check(
    z.refine((v) => v.trim().length > 0, { error: "必须填写理由" }),
    z.maxLength(500, { error: "理由最多 500 字" }),
  ),
});

/** 必须写理由的确认弹窗（写入审计日志并推送管理员群）。`onConfirm` 失败时错误留在弹窗里。 */
export function ReasonDialog({
  title,
  description,
  confirmLabel,
  danger = false,
  onConfirm,
  onClose,
}: {
  title: string;
  description: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: (reason: string) => Promise<unknown>;
  onClose: () => void;
}) {
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { reason: "" },
  });
  const run = useMutation({
    mutationFn: (reason: string) => onConfirm(reason),
    onSuccess: onClose,
  });
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent title={title} description={description}>
        <form
          className="space-y-3"
          noValidate
          onSubmit={form.handleSubmit((v) => run.mutate(v.reason.trim()))}
        >
          <Field label="理由（写入审计日志）" error={form.formState.errors.reason?.message}>
            <Textarea maxLength={500} className="h-20 resize-none" {...form.register("reason")} />
          </Field>
          {run.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(run.error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>
              取消
            </Button>
            <Button type="submit" variant={danger ? "danger" : "primary"} disabled={run.isPending}>
              {run.isPending ? "处理中…" : confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
