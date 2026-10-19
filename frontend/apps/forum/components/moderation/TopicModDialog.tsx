"use client";

import type { Schemas } from "@akari/api-client";
import { Field } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { usePreview } from "../forum/preview";
import { QueryProvider } from "../QueryProvider";
import { selectClass } from "../forum/styles";
import { ModDialog, ReasonFields } from "./ModDialog";
import { composeReason, PRESET_REASONS, reasonComplete, reasonFields } from "./reason";
import type { TopicActionItem } from "./topic-actions";

const Schema = z
  .object({ ...reasonFields, forumId: z.string() })
  .check(z.refine(reasonComplete, { error: "选「其他」时请写明原因", path: ["note"] }));
type Input = z.infer<typeof Schema>;

interface Props {
  topicId: number;
  item: TopicActionItem;
  /** 「移动」可选的目标板块（已按权限过滤、不含当前板块） */
  targets: Pick<Schemas["ForumView"], "id" | "name">[];
  onClose: () => void;
  onDone: () => void;
}

function TopicModForm({ topicId, item, targets, onClose, onDone }: Props) {
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { preset: PRESET_REASONS[0], note: "", forumId: String(targets[0]?.id ?? "") },
  });
  const preview = usePreview();
  const moderate = useMutation({
    mutationFn: (v: Input) =>
      // 设计稿：不提交
      preview
        ? Promise.resolve(undefined)
        : unwrap(
            api.POST("/api/v1/topics/{id}/moderation", {
              params: { path: { id: topicId } },
              body: {
                action: item.action,
                reason: composeReason(v),
                forum_id: item.action === "move" ? Number(v.forumId) : null,
              },
            }),
          ),
    onSuccess: onDone,
  });
  return (
    <ModDialog
      title={item.label}
      confirmLabel="确认"
      danger={item.danger}
      pending={moderate.isPending}
      error={moderate.error}
      onClose={onClose}
      onSubmit={form.handleSubmit((v) => moderate.mutate(v))}
    >
      {item.action === "move" ? (
        <Field label="目标板块">
          <select className={selectClass} {...form.register("forumId")}>
            {targets.map((f) => (
              <option key={f.id} value={f.id}>
                {f.name}
              </option>
            ))}
          </select>
        </Field>
      ) : null}
      <ReasonFields form={form} />
    </ModDialog>
  );
}

export default function TopicModDialog(props: Props) {
  return (
    <QueryProvider>
      <TopicModForm {...props} />
    </QueryProvider>
  );
}
