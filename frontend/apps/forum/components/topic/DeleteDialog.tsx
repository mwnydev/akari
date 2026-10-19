"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Field, Input } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { QueryProvider } from "../QueryProvider";

interface Props {
  postId: number;
  floor: number;
  /** 管理人员删除别人的楼层必须写理由（进审计日志） */
  needsReason: boolean;
  onClose: () => void;
  onDeleted: () => void;
}

function DeleteForm({ postId, floor, needsReason, onClose, onDeleted }: Props) {
  const [reason, setReason] = useState("");
  const del = useMutation({
    mutationFn: () =>
      unwrap(
        api.DELETE("/api/v1/posts/{id}", {
          params: { path: { id: postId }, query: needsReason ? { reason: reason.trim() } : {} },
        }),
      ),
    onSuccess: onDeleted,
  });
  const missing = needsReason && !reason.trim();
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent
        title={`删除 #${floor}`}
        description="删除后楼层号保留，显示「该楼层已被删除」；编辑历史仍保留给管理人员。"
      >
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (!missing) del.mutate();
          }}
        >
          {needsReason ? (
            <Field label="删除理由" hint="会写入管理日志">
              <Input value={reason} maxLength={500} onChange={(e) => setReason(e.target.value)} />
            </Field>
          ) : null}
          {del.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(del.error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>
              取消
            </Button>
            <Button type="submit" variant="danger" disabled={missing || del.isPending}>
              {del.isPending ? "删除中…" : "确认删除"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export default function DeleteDialog(props: Props) {
  return (
    <QueryProvider>
      <DeleteForm {...props} />
    </QueryProvider>
  );
}
