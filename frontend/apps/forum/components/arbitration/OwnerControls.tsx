"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Textarea } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { Pause, Play } from "lucide-react";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { QueryProvider } from "../QueryProvider";

const Schema = z.object({
  reason: z.string().check(
    z.refine((v) => v.trim().length > 0, { error: "请写明叫停理由" }),
    z.maxLength(2000, { error: "最多 2000 字" }),
  ),
});

function HaltDialog({ caseId, onClose }: { caseId: number; onClose: () => void }) {
  const router = useRouter();
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { reason: "" },
  });
  const halt = useMutation({
    mutationFn: (v: z.infer<typeof Schema>) =>
      unwrap(
        api.POST("/api/v1/arbitration/cases/{id}/halt", {
          params: { path: { id: caseId } },
          body: { reason: v.reason.trim() },
        }),
      ),
    onSuccess: () => {
      onClose();
      router.refresh();
    },
  });
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent
        title="叫停执行"
        description="公示后、仲裁 Safe 执行前可以叫停；之后可恢复，或以站长终裁改判（重新公示）。申诉中叫停的只能恢复后走复核。"
      >
        <form className="space-y-3" noValidate onSubmit={form.handleSubmit((v) => halt.mutate(v))}>
          <Field
            label="叫停理由（仲裁员与站长可见，写入审计日志）"
            error={form.formState.errors.reason?.message}
          >
            <Textarea maxLength={2000} {...form.register("reason")} />
          </Field>
          {halt.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(halt.error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={onClose}>
              取消
            </Button>
            <Button type="submit" variant="danger" disabled={halt.isPending}>
              叫停
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function Controls({ caseId, halt, resume }: { caseId: number; halt: boolean; resume: boolean }) {
  const router = useRouter();
  const [halting, setHalting] = useState(false);
  const doResume = useMutation({
    mutationFn: () =>
      unwrap(
        api.POST("/api/v1/arbitration/cases/{id}/resume", { params: { path: { id: caseId } } }),
      ),
    onSuccess: () => router.refresh(),
  });
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2">
        {halt ? (
          <Button variant="danger" onClick={() => setHalting(true)}>
            <Pause size={16} aria-hidden />
            叫停执行
          </Button>
        ) : null}
        {resume ? (
          <Button
            variant="secondary"
            onClick={() => doResume.mutate()}
            disabled={doResume.isPending}
          >
            <Play size={16} aria-hidden />
            解除叫停
          </Button>
        ) : null}
      </div>
      {doResume.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(doResume.error)}
        </p>
      ) : null}
      {halting ? <HaltDialog caseId={caseId} onClose={() => setHalting(false)} /> : null}
    </div>
  );
}

/** 站长的叫停 / 解除叫停（终裁走裁决表单）。 */
export default function OwnerControls(props: { caseId: number; halt: boolean; resume: boolean }) {
  return (
    <QueryProvider>
      <Controls {...props} />
    </QueryProvider>
  );
}
