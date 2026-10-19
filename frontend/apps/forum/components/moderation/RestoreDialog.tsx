"use client";

import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { QueryProvider } from "../QueryProvider";
import { ModDialog, ReasonFields } from "./ModDialog";
import { composeReason, reasonComplete, reasonFields } from "./reason";

const Schema = z
  .object(reasonFields)
  .check(z.refine(reasonComplete, { error: "选「其他」时请写明原因", path: ["note"] }));

interface Props {
  postId: number;
  floor: number;
  onClose: () => void;
  onDone: () => void;
}

function RestoreForm({ postId, floor, onClose, onDone }: Props) {
  const form = useForm<z.infer<typeof Schema>>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { preset: "误操作恢复", note: "" },
  });
  const restore = useMutation({
    mutationFn: (v: z.infer<typeof Schema>) =>
      unwrap(
        api.POST("/api/v1/posts/{id}/restore", {
          params: { path: { id: postId } },
          body: { reason: composeReason(v) },
        }),
      ),
    onSuccess: onDone,
  });
  return (
    <ModDialog
      title={`恢复 #${floor}`}
      confirmLabel="恢复"
      pending={restore.isPending}
      error={restore.error}
      onClose={onClose}
      onSubmit={form.handleSubmit((v) => restore.mutate(v))}
    >
      <ReasonFields form={form} />
    </ModDialog>
  );
}

export default function RestoreDialog(props: Props) {
  return (
    <QueryProvider>
      <RestoreForm {...props} />
    </QueryProvider>
  );
}
