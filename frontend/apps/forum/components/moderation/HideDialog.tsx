"use client";

import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { QueryProvider } from "../QueryProvider";
import { ModDialog, ReasonFields } from "./ModDialog";
import { composeReason, PRESET_REASONS, reasonComplete, reasonFields } from "./reason";

const Schema = z
  .object(reasonFields)
  .check(z.refine(reasonComplete, { error: "选「其他」时请写明原因", path: ["note"] }));

interface Props {
  postId: number;
  floor: number;
  /** `true` 取消隐藏 */
  unhide: boolean;
  onClose: () => void;
  onDone: () => void;
}

function HideForm({ postId, floor, unhide, onClose, onDone }: Props) {
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { preset: unhide ? "误操作恢复" : PRESET_REASONS[0], note: "" },
  });
  const submit = useMutation({
    mutationFn: (v: z.infer<typeof Schema>) => {
      const req = { params: { path: { id: postId } }, body: { reason: composeReason(v) } };
      return unwrap(
        unhide
          ? api.POST("/api/v1/posts/{id}/unhide", req)
          : api.POST("/api/v1/posts/{id}/hide", req),
      );
    },
    onSuccess: onDone,
  });
  return (
    <ModDialog
      title={unhide ? `取消隐藏 #${floor}` : `隐藏 #${floor}`}
      description={
        unhide
          ? "取消后所有人都能看到这层楼；理由写进管理日志"
          : "隐藏后只有管理人员和作者看得到内容，作者会看到理由；不能再点赞、评论。理由写进管理日志，可由站长撤销"
      }
      confirmLabel={unhide ? "取消隐藏" : "隐藏"}
      danger={!unhide}
      pending={submit.isPending}
      error={submit.error}
      onClose={onClose}
      onSubmit={form.handleSubmit((v) => submit.mutate(v))}
    >
      <ReasonFields form={form} />
    </ModDialog>
  );
}

export default function HideDialog(props: Props) {
  return (
    <QueryProvider>
      <HideForm {...props} />
    </QueryProvider>
  );
}
