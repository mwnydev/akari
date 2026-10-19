"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Button, Field, Input } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/browser-api";
import { QueryProvider } from "../QueryProvider";

type Action = Schemas["ModerationAction"];

interface Props {
  topicId: number;
  forumId: number;
  flags: Schemas["TopicFlags"];
  forums: Pick<Schemas["ForumView"], "id" | "name">[];
  onClose: () => void;
  onDone: () => void;
}

/** 预设理由（plan 第 9 节），可再补充说明。 */
const PRESET_REASONS = ["违反版规", "广告或引流", "内容重复", "优质内容", "其他"];

function actionsFor(flags: Schemas["TopicFlags"]): [Action, string][] {
  return [
    flags.pinned ? ["unpin", "取消置顶"] : ["pin", "置顶"],
    flags.featured ? ["unfeature", "取消精华"] : ["feature", "设为精华"],
    flags.locked ? ["unlock", "解除锁定"] : ["lock", "锁定（禁止回复）"],
    flags.hidden ? ["unhide", "取消隐藏"] : ["hide", "隐藏"],
    ["move", "移动到其他板块"],
  ];
}

const ACTIONS = [
  "pin",
  "unpin",
  "feature",
  "unfeature",
  "lock",
  "unlock",
  "hide",
  "unhide",
  "move",
] as const satisfies readonly Action[];

const Schema = z.object({
  action: z.enum(ACTIONS),
  preset: z.string(),
  note: z.string().check(z.maxLength(400, { error: "补充说明最多 400 字" })),
  forumId: z.string(),
});
type Input = z.infer<typeof Schema>;

const selectClass = "h-10 w-full rounded-md border border-border bg-surface px-3 text-fg";

function ModerationForm({ topicId, forumId, flags, forums, onClose, onDone }: Props) {
  const actions = actionsFor(flags);
  const targets = forums.filter((f) => f.id !== forumId);
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: {
      action: actions[0]?.[0] ?? "pin",
      preset: PRESET_REASONS[0] ?? "",
      note: "",
      forumId: String(targets[0]?.id ?? ""),
    },
  });
  const action = form.watch("action");

  const moderate = useMutation({
    mutationFn: (v: Input) => {
      const note = v.note.trim();
      return unwrap(
        api.POST("/api/v1/topics/{id}/moderation", {
          params: { path: { id: topicId } },
          body: {
            action: v.action,
            reason: note ? `${v.preset}：${note}` : v.preset,
            forum_id: v.action === "move" ? Number(v.forumId) : null,
          },
        }),
      );
    },
    onSuccess: onDone,
  });

  const noteError = form.formState.errors.note?.message;
  return (
    <Dialog open onOpenChange={(o) => (o ? undefined : onClose())}>
      <DialogContent title="主题管理" description="每次操作都会连同理由写入管理日志">
        <form
          className="space-y-3"
          noValidate
          onSubmit={form.handleSubmit((v) => moderate.mutate(v))}
        >
          <Field label="操作">
            <select className={selectClass} {...form.register("action")}>
              {actions.map(([value, label]) => (
                <option key={value} value={value} disabled={value === "move" && !targets.length}>
                  {label}
                </option>
              ))}
            </select>
          </Field>
          {action === "move" ? (
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
          <Field label="理由">
            <select className={selectClass} {...form.register("preset")}>
              {PRESET_REASONS.map((r) => (
                <option key={r}>{r}</option>
              ))}
            </select>
          </Field>
          <Field label="补充说明（可选）" error={noteError}>
            <Input maxLength={400} {...form.register("note")} />
          </Field>
          {moderate.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(moderate.error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>
              取消
            </Button>
            <Button type="submit" disabled={moderate.isPending}>
              {moderate.isPending ? "提交中…" : "确认"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export default function ModerationDialog(props: Props) {
  return (
    <QueryProvider>
      <ModerationForm {...props} />
    </QueryProvider>
  );
}
