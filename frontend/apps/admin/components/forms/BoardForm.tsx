"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Card, CardTitle, Field, Input } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/api";
import { boardPatch, boardValues, PALETTE, type Board, type BoardValues } from "../../lib/boards";
import { keys } from "../../lib/queries";
import { useAdminSession } from "../session/SessionProvider";

// 与后端 routes::admin_forums 的上限一致（服务端仍会再校验）
const Schema = z.object({
  name: z.string().check(
    z.refine((s) => s.trim().length > 0, { error: "请填写名称" }),
    z.maxLength(20, { error: "最多 20 字" }),
  ),
  description: z.string().check(z.maxLength(200, { error: "最多 200 字" })),
  color: z.string(),
  position: z.coerce
    .number({ error: "请输入整数" })
    .check(
      z.refine(Number.isInteger, { error: "请输入整数" }),
      z.gte(0, { error: "不能小于 0" }),
      z.lte(9999, { error: "不能大于 9999" }),
    ),
  allowTrade: z.boolean(),
});

const selectClass =
  "h-10 w-full rounded-md border border-border bg-surface px-3 text-sm text-fg focus-visible:outline-2 focus-visible:outline-accent";

/** 编辑一个板块（表单库按需加载）。只提交改了的字段；改「能否发交易帖」涉及钱，先用后台 Passkey 再验证。 */
export default function BoardForm({ board, onDone }: { board: Board; onDone: () => void }) {
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const form = useForm<BoardValues>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: boardValues(board),
  });
  const save = useMutation({
    mutationFn: async (v: BoardValues) => {
      const patch = boardPatch(board, v);
      if (!patch) return null;
      const run = () =>
        unwrap(
          api.PATCH("/api/v1/admin/forums/{id}", {
            params: { path: { id: board.id } },
            body: patch.body,
          }),
        );
      return patch.tradeChanged ? sensitive(run) : run();
    },
    onSuccess: (saved) => {
      if (saved) {
        void queryClient.invalidateQueries({ queryKey: keys.forums });
        toast.success("已保存，并已推送管理员群");
      }
      onDone();
    },
  });
  const errors = form.formState.errors;
  return (
    <Card>
      <CardTitle>编辑「{board.name}」</CardTitle>
      <form
        className="mt-3 space-y-3"
        noValidate
        aria-label="编辑板块"
        onSubmit={form.handleSubmit((v) => save.mutate(v))}
      >
        <div className="grid gap-3 md:grid-cols-2">
          <Field label="名称" error={errors.name?.message}>
            <Input maxLength={20} {...form.register("name")} />
          </Field>
          <Field label="排序" hint="同级板块里小的在前，0–9999" error={errors.position?.message}>
            <Input type="number" inputMode="numeric" {...form.register("position")} />
          </Field>
          <Field label="简介" error={errors.description?.message}>
            <Input maxLength={200} {...form.register("description")} />
          </Field>
          <Field label="色块">
            <select className={selectClass} {...form.register("color")}>
              <option value="">按排序位置轮换</option>
              {PALETTE.map((p) => (
                <option key={p.hex} value={p.hex}>
                  {p.label} {p.hex}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" {...form.register("allowTrade")} />
          允许发交易帖（修改需要后台 Passkey 再验证）
        </label>
        {save.isError ? (
          <p role="alert" className="text-sm text-danger">
            {errorMessage(save.error)}
          </p>
        ) : null}
        <div className="flex gap-2">
          <Button type="submit" disabled={save.isPending}>
            {save.isPending ? "保存中…" : "保存"}
          </Button>
          <Button variant="secondary" onClick={onDone}>
            取消
          </Button>
        </div>
      </form>
    </Card>
  );
}
