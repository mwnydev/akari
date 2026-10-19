"use client";

import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { QueryProvider } from "../QueryProvider";
import { ModDialog, ReasonFields } from "./ModDialog";
import { composeReason, PRESET_REASONS, reasonComplete, reasonFields } from "./reason";

const Schema = z
  .object(reasonFields)
  .check(z.refine(reasonComplete, { error: "选「其他」时请写明原因", path: ["note"] }));
type Input = z.infer<typeof Schema>;

interface Props {
  title: string;
  confirmLabel: string;
  danger?: boolean;
  /** 默认选中的预设理由 */
  preset?: (typeof PRESET_REASONS)[number];
  /** 带着整理好的理由去请求；失败时抛错，弹窗里显示 */
  onConfirm: (reason: string) => Promise<void>;
  onClose: () => void;
}

/** 楼中评论的管理操作（删除别人的评论、恢复）：必须选理由，成功后关闭。 */
function CommentModForm({ title, confirmLabel, danger, preset, onConfirm, onClose }: Props) {
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { preset: preset ?? PRESET_REASONS[0], note: "" },
  });
  const run = useMutation({
    mutationFn: (v: Input) => onConfirm(composeReason(v)),
    onSuccess: onClose,
  });
  return (
    <ModDialog
      title={title}
      confirmLabel={confirmLabel}
      danger={danger ?? false}
      pending={run.isPending}
      error={run.error}
      onClose={onClose}
      onSubmit={form.handleSubmit((v) => run.mutate(v))}
    >
      <ReasonFields form={form} />
    </ModDialog>
  );
}

export default function CommentModDialog(props: Props) {
  return (
    <QueryProvider>
      <CommentModForm {...props} />
    </QueryProvider>
  );
}
