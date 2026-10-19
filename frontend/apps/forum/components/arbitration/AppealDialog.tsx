"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Textarea } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";

const Schema = z.object({
  reason: z.string().check(
    z.refine((v) => v.trim().length > 0, { error: "请写明申诉理由" }),
    z.maxLength(2000, { error: "最多 2000 字" }),
  ),
});

/** 申诉：裁决公示后 72 小时内、每个案件一次；由没参与过的人复核，复核结果为最终裁决。 */
export default function AppealDialog({
  caseId,
  onClose,
  onDone,
}: {
  caseId: number;
  onClose: () => void;
  onDone: () => void;
}) {
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { reason: "" },
  });
  const appeal = useMutation({
    mutationFn: (v: z.infer<typeof Schema>) =>
      unwrap(
        api.POST("/api/v1/arbitration/cases/{id}/appeal", {
          params: { path: { id: caseId } },
          body: { reason: v.reason.trim() },
        }),
      ),
    onSuccess: onDone,
  });
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent
        title="申诉"
        description="每个案件只能申诉一次。复核由没参与过本案的人进行，复核结果为最终裁决。"
      >
        <form
          className="space-y-3"
          noValidate
          onSubmit={form.handleSubmit((v) => appeal.mutate(v))}
        >
          <Field label="申诉理由" error={form.formState.errors.reason?.message}>
            <Textarea maxLength={2000} {...form.register("reason")} />
          </Field>
          {appeal.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(appeal.error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={onClose}>
              取消
            </Button>
            <Button type="submit" disabled={appeal.isPending}>
              {appeal.isPending ? "提交中…" : "提交申诉"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
