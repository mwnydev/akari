"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Button, Card, Field, Input } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/api";
import { keys } from "../../lib/queries";
import { useAdminSession } from "../session/SessionProvider";

type Settings = Schemas["ModerationSettings"];
type Key = keyof Settings;

const FIELDS: { key: Key; label: string; unit: "秒" | "次" }[] = [
  { key: "rate_window_secs", label: "限速窗口", unit: "秒" },
  { key: "rate_content", label: "窗口内最多：置顶 / 精华 / 锁定 / 移动 / 编辑 / 恢复", unit: "次" },
  { key: "rate_delete", label: "窗口内最多：删帖、隐藏主题", unit: "次" },
  { key: "rate_sanction", label: "窗口内最多：禁言、禁拍、封号、封 IP / 设备及解除", unit: "次" },
  { key: "anomaly_window_secs", label: "异常检测窗口", unit: "秒" },
  { key: "anomaly_deletes", label: "窗口内删帖 + 隐藏超过即暂停管理权限", unit: "次" },
  { key: "anomaly_sanctions", label: "窗口内处罚超过即暂停管理权限", unit: "次" },
];

function bounded(lo: number, hi: number) {
  return z.coerce
    .number({ error: "请输入整数" })
    .check(
      z.refine(Number.isInteger, { error: "请输入整数" }),
      z.gte(lo, { error: `不能小于 ${lo}` }),
      z.lte(hi, { error: `不能大于 ${hi}` }),
    );
}

/** 表单校验与服务端上下限一致（服务端仍会再校验）。 */
function schemaFor(min: Settings, max: Settings) {
  return z.object(
    Object.fromEntries(FIELDS.map(({ key }) => [key, bounded(min[key], max[key])])) as Record<
      Key,
      ReturnType<typeof bounded>
    >,
  );
}

/** 限速与异常阈值表单（表单库按需加载，不进页面首屏 JS）。 */
export default function SettingsForm({ view }: { view: Schemas["ModerationSettingsView"] }) {
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const schema = useMemo(() => schemaFor(view.min, view.max), [view.min, view.max]);
  const form = useForm<Settings>({
    resolver: standardSchemaResolver(schema),
    defaultValues: view.settings,
  });
  const save = useMutation({
    mutationFn: (body: Settings) =>
      sensitive(() => unwrap(api.PUT("/api/v1/admin/settings/moderation", { body }))),
    onSuccess: (next) => {
      queryClient.setQueryData(keys.settings, next);
      form.reset(next.settings);
      toast.success("已保存，并已推送管理员群");
    },
  });
  return (
    <Card>
      <form
        className="space-y-3"
        noValidate
        onSubmit={form.handleSubmit((v) => save.mutate(v))}
        aria-label="管理限速与异常阈值"
      >
        <div className="grid gap-3 md:grid-cols-2">
          {FIELDS.map(({ key, label, unit }) => (
            <Field
              key={key}
              label={label}
              hint={`${view.min[key]}–${view.max[key]} ${unit}，默认 ${view.defaults[key]}`}
              error={form.formState.errors[key]?.message}
            >
              <Input type="number" inputMode="numeric" {...form.register(key)} />
            </Field>
          ))}
        </div>
        {save.isError ? (
          <p role="alert" className="text-sm text-danger">
            {errorMessage(save.error)}
          </p>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button type="submit" disabled={save.isPending || !form.formState.isDirty}>
            {save.isPending ? "保存中…" : "保存"}
          </Button>
          <Button
            variant="secondary"
            onClick={() => form.reset(view.defaults, { keepDefaultValues: true })}
          >
            填入默认值
          </Button>
        </div>
      </form>
    </Card>
  );
}
