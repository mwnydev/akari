"use client";

import { type Schemas } from "@akari/api-client";
import { Field, Input } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { formatTime } from "../../lib/display";
import {
  SANCTION_LABEL,
  durationChoices,
  durationLimit,
  sanctionChoices,
  type SanctionKind,
} from "../../lib/staff";
import { QueryProvider } from "../QueryProvider";
import { selectClass } from "../forum/styles";
import { ModDialog, ReasonFields } from "./ModDialog";
import { composeReason, PRESET_REASONS, reasonComplete, reasonFields } from "./reason";

const KINDS = ["mute", "auction_ban", "ban", "ip_ban", "device_ban"] as const;

const Schema = z
  .object({
    ...reasonFields,
    kind: z.enum(KINDS),
    scope: z.enum(["board", "site"]),
    /** 时长档位的秒数；"" = 永久 */
    duration: z.string(),
    ip: z.string(),
  })
  .check(
    z.refine(reasonComplete, { error: "选「其他」时请写明原因", path: ["note"] }),
    z.refine((v) => v.kind !== "ip_ban" || v.ip.trim().length > 0, {
      error: "请填写 IP 或网段",
      path: ["ip"],
    }),
  );
type Input = z.infer<typeof Schema>;

interface Props {
  user: { id: number; name: string };
  /** 在哪个板块里处罚（版主只能限这个板块） */
  forumId: number;
  staff: Schemas["StaffView"];
  onClose: () => void;
}

function SanctionForm({ user, forumId, staff, onClose }: Props) {
  const choices = sanctionChoices(staff, forumId);
  const first = choices[0];
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: {
      preset: PRESET_REASONS[0],
      note: "",
      kind: first?.kind ?? "mute",
      scope: first?.board ? "board" : "site",
      duration: "86400",
      ip: "",
    },
  });
  const [done, setDone] = useState<Schemas["SanctionsCreated"] | null>(null);
  const kind = form.watch("kind") as SanctionKind;
  const choice = choices.find((c) => c.kind === kind);
  const durations = durationChoices(durationLimit(staff, kind));

  const submit = useMutation({
    mutationFn: (v: Input) => {
      const reason = composeReason(v);
      const duration_secs = v.duration === "" ? null : Number(v.duration);
      if (v.kind === "ip_ban") {
        return unwrap(
          api.POST("/api/v1/moderation/ip-bans", {
            body: { ip: v.ip.trim(), duration_secs, reason },
          }),
        );
      }
      return unwrap(
        api.POST("/api/v1/moderation/users/{id}/sanctions", {
          params: { path: { id: user.id } },
          body: {
            kind: v.kind,
            forum_id: v.scope === "board" ? forumId : null,
            duration_secs,
            reason,
          },
        }),
      );
    },
    onSuccess: setDone,
  });

  if (done) {
    return (
      <ModDialog
        title={`已${SANCTION_LABEL[kind]}`}
        description="已写入管理日志并通知对方"
        confirmLabel="完成"
        pending={false}
        error={null}
        onSubmit={onClose}
        onClose={onClose}
      >
        <p className="text-sm" role="status">
          {done.ends_at ? `到期时间：${formatTime(done.ends_at)}` : "永久有效，直到被解除"}
        </p>
      </ModDialog>
    );
  }

  return (
    <ModDialog
      title={`处罚 ${user.name}`}
      confirmLabel="确认处罚"
      danger
      pending={submit.isPending}
      error={submit.error}
      onClose={onClose}
      onSubmit={form.handleSubmit((v) => submit.mutate(v))}
    >
      <Field label="处罚">
        <select
          className={selectClass}
          {...form.register("kind", {
            onChange: () => {
              const c = choices.find((x) => x.kind === form.getValues("kind"));
              form.setValue("scope", c?.board ? "board" : "site");
              form.setValue("duration", "86400");
            },
          })}
        >
          {choices.map((c) => (
            <option key={c.kind} value={c.kind}>
              {SANCTION_LABEL[c.kind]}
            </option>
          ))}
        </select>
      </Field>
      {choice?.board && choice.site ? (
        <Field label="范围">
          <select className={selectClass} {...form.register("scope")}>
            <option value="board">只限本板块</option>
            <option value="site">全站</option>
          </select>
        </Field>
      ) : null}
      {kind === "ip_ban" ? (
        <Field
          label="IP 或网段"
          hint="如 203.0.113.7 或 203.0.113.0/24（IPv4 最大 /16、IPv6 最大 /32）；只拦登录"
          error={form.formState.errors.ip?.message}
        >
          <Input {...form.register("ip")} />
        </Field>
      ) : null}
      <Field
        label="时长"
        {...(durations.at(-1)?.secs === null
          ? {}
          : { hint: `你的角色最长 ${durations.at(-1)?.label ?? ""}，更长或封号请交管理员处理` })}
      >
        <select className={selectClass} {...form.register("duration")}>
          {durations.map((d) => (
            <option key={d.label} value={d.secs ?? ""}>
              {d.label}
            </option>
          ))}
        </select>
      </Field>
      {kind === "ban" ? (
        <p className="text-sm text-fg-muted">
          封号后不能发帖、回帖、编辑和发私信；仍可登录查看订单与钱包、提币（托管资金永远能取回）。
        </p>
      ) : null}
      <ReasonFields form={form} />
    </ModDialog>
  );
}

export default function SanctionDialog(props: Props) {
  return (
    <QueryProvider>
      <SanctionForm {...props} />
    </QueryProvider>
  );
}
