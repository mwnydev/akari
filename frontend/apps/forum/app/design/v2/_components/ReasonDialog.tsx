"use client";

import { Button, Field, Textarea } from "@akari/ui";
import { Dialog, DialogClose, DialogContent } from "@akari/ui/dialog";
import type { ReactNode } from "react";
import { selectClass } from "./styles";

/** 与正式管理弹窗一致的预设理由（plan 第 9 节），可再补充说明。 */
const PRESET_REASONS = ["违反版规", "广告或引流", "内容重复", "优质内容", "其他"];

/** 管理操作确认：必须选理由（写进审计日志并通知作者），`children` 放操作特有的选项（目标板块、禁言时长）。 */
export default function ReasonDialog({
  open,
  onOpenChange,
  title,
  confirmLabel,
  danger = false,
  onConfirm,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm?: () => void;
  children?: ReactNode;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={title} description="理由会写进管理日志并通知作者，操作可撤销">
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            onConfirm?.();
            onOpenChange(false);
          }}
        >
          {children}
          <Field label="理由">
            <select className={selectClass} defaultValue={PRESET_REASONS[0]}>
              {PRESET_REASONS.map((r) => (
                <option key={r}>{r}</option>
              ))}
            </select>
          </Field>
          <Field label="补充说明（可选）">
            <Textarea maxLength={400} className="h-20 resize-none" />
          </Field>
          <div className="flex justify-end gap-2 pt-1">
            <DialogClose asChild>
              <Button variant="secondary">取消</Button>
            </DialogClose>
            <Button type="submit" variant={danger ? "danger" : "primary"}>
              {confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
