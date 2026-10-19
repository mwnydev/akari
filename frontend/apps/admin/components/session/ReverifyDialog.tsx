"use client";

import { errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { useMutation } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { reverifyWithPasskey } from "../../lib/passkey";
import { useAdminSession } from "./SessionProvider";

/** 敏感操作前的后台 Passkey 再验证；成功后 5 分钟内的敏感操作不再询问。 */
export function ReverifyDialog({ onDone }: { onDone: (ok: boolean) => void }) {
  const { adopt } = useAdminSession();
  const verify = useMutation({
    mutationFn: reverifyWithPasskey,
    onSuccess: (view) => {
      adopt(view);
      onDone(true);
    },
  });
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onDone(false))}>
      <DialogContent
        title="再次验证后台 Passkey"
        description="任免、改阈值、恢复权限、执行撤销等敏感操作前需要再验证一次，5 分钟内可连续处理。"
      >
        {verify.isError ? (
          <p role="alert" className="mb-3 text-sm text-danger">
            {errorMessage(verify.error)}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => onDone(false)}>
            取消
          </Button>
          <Button onClick={() => verify.mutate()} disabled={verify.isPending}>
            <KeyRound size={16} aria-hidden />
            {verify.isPending ? "等待 Passkey…" : "用 Passkey 验证"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
