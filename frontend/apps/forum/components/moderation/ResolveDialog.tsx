"use client";

import type { Schemas } from "@akari/api-client";
import { Field, Textarea } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { userSanctionChoices, type SanctionKind } from "../../lib/staff";
import { QueryProvider } from "../QueryProvider";
import { ModDialog, ReasonFields } from "./ModDialog";
import { composeReason, PRESET_REASONS, reasonComplete, reasonFields } from "./reason";
import { SanctionFields } from "./SanctionDialog";

export type ResolveAction = Schemas["ResolveAction"];

const Schema = z
  .object({
    ...reasonFields,
    action: z.enum(["dismiss", "hide", "delete", "sanction"]),
    kind: z.enum(["mute", "auction_ban", "ban", "ip_ban", "device_ban"]),
    scope: z.enum(["board", "site"]),
    duration: z.string(),
    ip: z.string(),
  })
  .check(
    z.refine((v) => (v.action === "dismiss" ? v.note.trim().length > 0 : reasonComplete(v)), {
      error: "请写明原因",
      path: ["note"],
    }),
  );
type Input = z.infer<typeof Schema>;

export interface ResolveTarget {
  kind: Schemas["ReportTargetKind"];
  floor: number;
  /** 内容现在所在的板块 */
  forumId: number;
}

interface Props {
  caseId: number;
  action: ResolveAction;
  target: ResolveTarget;
  author: { id: number; name: string };
  staff: Schemas["StaffView"];
  onClose: () => void;
  onDone: () => void;
}

const TITLE: Record<ResolveAction, (p: Props) => string> = {
  dismiss: () => "驳回举报（未发现违规）",
  hide: (p) => (p.target.floor === 1 ? "隐藏主题" : `隐藏 #${p.target.floor}`),
  delete: (p) => (p.target.kind === "post" ? `删除 #${p.target.floor}` : "删除评论"),
  sanction: (p) => `处罚 ${p.author.name}`,
};

const CONFIRM: Record<ResolveAction, string> = {
  dismiss: "驳回",
  hide: "隐藏并结案",
  delete: "删除并结案",
  sanction: "处罚并结案",
};

function ResolveForm(props: Props) {
  const { caseId, action, target, staff, onClose, onDone } = props;
  const choices = userSanctionChoices(staff, target.forumId);
  const first = choices[0];
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: {
      action,
      preset: PRESET_REASONS[0],
      note: "",
      kind: first?.kind ?? "mute",
      scope: first?.board ? "board" : "site",
      duration: "86400",
      ip: "",
    },
  });
  const resolve = useMutation({
    mutationFn: (v: Input) =>
      unwrap(
        api.POST("/api/v1/moderation/reports/{id}/resolve", {
          params: { path: { id: caseId } },
          body: {
            action,
            reason: action === "dismiss" ? v.note.trim() : composeReason(v),
            sanction:
              action === "sanction"
                ? {
                    kind: v.kind as SanctionKind,
                    forum_id: v.scope === "board" ? target.forumId : null,
                    duration_secs: v.duration === "" ? null : Number(v.duration),
                  }
                : null,
          },
        }),
      ),
    onSuccess: onDone,
  });
  return (
    <ModDialog
      title={TITLE[action](props)}
      description={
        action === "dismiss"
          ? "说明只写进管理日志；举报人只会收到「未发现违规」的通知"
          : "理由写进管理日志，可由站长撤销；举报人只会收到「已处理」的通知，看不到处理方式"
      }
      confirmLabel={CONFIRM[action]}
      danger={action !== "dismiss"}
      pending={resolve.isPending}
      error={resolve.error}
      onClose={onClose}
      onSubmit={form.handleSubmit((v) => resolve.mutate(v))}
    >
      {action === "sanction" ? (
        <SanctionFields form={form} choices={choices} staff={staff} />
      ) : null}
      {action === "dismiss" ? (
        <Field label="说明" error={form.formState.errors.note?.message}>
          <Textarea maxLength={400} className="h-20 resize-none" {...form.register("note")} />
        </Field>
      ) : (
        <ReasonFields form={form} />
      )}
    </ModDialog>
  );
}

export default function ResolveDialog(props: Props) {
  return (
    <QueryProvider>
      <ResolveForm {...props} />
    </QueryProvider>
  );
}
