"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Badge, Button, Card, Field, Input, Textarea } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Controller, useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/api";
import { actionLabel, formatTime, REVERT_STATUS } from "../../lib/format";
import { MAX_REVOKE_SPAN_MS, revokeWindow } from "../../lib/revoke";
import { Section } from "../Section";
import { useAdminSession } from "../session/SessionProvider";
import { EmptyRow, Table, Td } from "../Table";
import { UserPicker } from "../UserPicker";

type Request = Schemas["RevokeRequest"];
type Result = Schemas["RevokeResult"];

const Schema = z
  .object({
    actor: z.nullable(z.object({ id: z.number(), label: z.string() })),
    from: z.string().check(z.minLength(1, { error: "请选择开始时间" })),
    to: z.string().check(z.minLength(1, { error: "请选择结束时间" })),
    reason: z.string().check(
      z.refine((v) => v.trim().length > 0, { error: "必须填写理由" }),
      z.maxLength(500, { error: "理由最多 500 字" }),
    ),
  })
  .check(
    z.refine((v) => v.actor !== null, { error: "请选择账号", path: ["actor"] }),
    z.refine((v) => revokeWindow(v.from, v.to) !== null, {
      error: `结束时间须晚于开始时间，且相隔不超过 ${MAX_REVOKE_SPAN_MS / 86_400_000} 天`,
      path: ["to"],
    }),
  );
type Input = z.infer<typeof Schema>;

function Items({ result }: { result: Result }) {
  return (
    <Table label="撤销明细" head={["日志", "时间", "操作", "对象", "状态"]}>
      {result.items.length === 0 ? <EmptyRow span={5}>这段时间内没有管理操作</EmptyRow> : null}
      {result.items.map((it) => (
        <tr key={it.log_id}>
          <Td className="amount">#{it.log_id}</Td>
          <Td className="whitespace-nowrap">{formatTime(it.created_at)}</Td>
          <Td>{actionLabel(it.action)}</Td>
          <Td className="amount">{it.target ?? "—"}</Td>
          <Td>
            <Badge tone={it.status === "reverted" ? "accent" : "neutral"}>
              {result.executed && it.status === "reverted" ? "已撤销" : REVERT_STATUS[it.status]}
            </Badge>
          </Td>
        </tr>
      ))}
    </Table>
  );
}

/** 一键撤销某账号某时间段的管理操作（ADR 0013 第 6 条）：先预览（服务端在回滚的事务里试做），确认后执行。 */
export function RevokePage() {
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { actor: null, from: "", to: "", reason: "" },
  });
  // 执行时用的正是预览过的那组参数；表单改了就作废预览
  const [preview, setPreview] = useState<{ request: Request; result: Result } | null>(null);
  const [done, setDone] = useState<Result | null>(null);

  const toRequest = (v: Input, execute: boolean): Request => {
    const w = revokeWindow(v.from, v.to);
    return {
      actor_id: v.actor?.id ?? 0,
      from: w?.from ?? "",
      to: w?.to ?? "",
      reason: v.reason.trim(),
      execute,
    };
  };

  const run = useMutation({
    mutationFn: async (body: Request) =>
      body.execute
        ? sensitive(() => unwrap(api.POST("/api/v1/admin/revoke", { body })))
        : unwrap(api.POST("/api/v1/admin/revoke", { body })),
    onSuccess: (result, body) => {
      if (body.execute) {
        setPreview(null);
        setDone(result);
        toast.success(`已撤销 ${result.reverted} 条管理操作`);
        void queryClient.invalidateQueries({ queryKey: ["admin-audit"] });
      } else {
        setDone(null);
        setPreview({ request: body, result });
      }
    },
  });

  const errors = form.formState.errors;
  return (
    <>
      <Section
        title="批量撤销管理操作"
        description="账号被盗或误操作时，撤销某账号在一段时间（最长 31 天）内的全部管理操作。只还原仍是该操作结果的状态，之后又被改过的会跳过；一次最多 500 条。"
      >
        <Card>
          <form
            className="grid gap-3 md:grid-cols-2"
            noValidate
            aria-label="批量撤销"
            onChange={() => setPreview(null)}
            onSubmit={form.handleSubmit((v) => run.mutate(toRequest(v, false)))}
          >
            <Controller
              control={form.control}
              name="actor"
              render={({ field }) => (
                <UserPicker
                  label="账号"
                  value={field.value}
                  onChange={(u) => {
                    field.onChange(u);
                    setPreview(null);
                  }}
                  error={errors.actor?.message}
                />
              )}
            />
            <div className="grid grid-cols-2 gap-3">
              <Field label="开始（含）" error={errors.from?.message}>
                <Input type="datetime-local" {...form.register("from")} />
              </Field>
              <Field label="结束（不含）" error={errors.to?.message}>
                <Input type="datetime-local" {...form.register("to")} />
              </Field>
            </div>
            <div className="md:col-span-2">
              <Field label="理由（写入审计日志并推送管理员群）" error={errors.reason?.message}>
                <Textarea
                  maxLength={500}
                  className="h-16 resize-none"
                  {...form.register("reason")}
                />
              </Field>
            </div>
            <div className="flex flex-wrap gap-2 md:col-span-2">
              <Button type="submit" variant="secondary" disabled={run.isPending}>
                预览
              </Button>
              <Button
                variant="danger"
                disabled={!preview || preview.result.reverted === 0 || run.isPending}
                onClick={() => preview && run.mutate({ ...preview.request, execute: true })}
              >
                {preview ? `执行撤销（${preview.result.reverted} 条）` : "执行撤销"}
              </Button>
            </div>
          </form>
          {run.isError ? (
            <p role="alert" className="mt-2 text-sm text-danger">
              {errorMessage(run.error)}
            </p>
          ) : null}
        </Card>
      </Section>
      {preview ? (
        <Section
          title={`预览：可撤销 ${preview.result.reverted} / ${preview.result.items.length} 条`}
          description={
            preview.result.truncated
              ? "超过 500 条，只列出最新的 500 条；执行后缩小时间段再做一次。"
              : "这只是预览，还没有改动任何内容。"
          }
        >
          <Items result={preview.result} />
        </Section>
      ) : null}
      {done ? (
        <Section title={`已执行：撤销 ${done.reverted} 条`}>
          <Items result={done} />
        </Section>
      ) : null}
    </>
  );
}
