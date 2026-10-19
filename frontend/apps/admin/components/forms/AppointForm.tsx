"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Card, Field } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Controller, useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/api";
import { ROLE_LABEL } from "../../lib/format";
import { keys, useForums } from "../../lib/queries";
import { useAdminSession } from "../session/SessionProvider";
import { UserPicker } from "../UserPicker";

const selectClass = "h-10 w-full rounded-md border border-border bg-surface px-3 text-fg";

const AppointSchema = z
  .object({
    user: z.nullable(z.object({ id: z.number(), label: z.string() })),
    role: z.enum(["admin", "moderator"]),
    forumId: z.string(),
  })
  .check(
    z.refine((v) => v.user !== null, { error: "请选择用户", path: ["user"] }),
    z.refine((v) => v.role !== "moderator" || v.forumId !== "", {
      error: "版主必须指定负责的板块",
      path: ["forumId"],
    }),
  );
type AppointInput = z.infer<typeof AppointSchema>;

/** 任命管理人员（表单库按需加载，不进页面首屏 JS）。 */
export default function AppointForm() {
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const forums = useForums();
  const form = useForm<AppointInput>({
    resolver: standardSchemaResolver(AppointSchema),
    defaultValues: { user: null, role: "moderator", forumId: "" },
  });
  const role = form.watch("role");
  const appoint = useMutation({
    mutationFn: (v: AppointInput) =>
      sensitive(() =>
        unwrap(
          api.POST("/api/v1/admin/staff", {
            body: {
              user_id: v.user?.id ?? 0,
              role: v.role,
              forum_id: v.role === "moderator" ? Number(v.forumId) : null,
            },
          }),
        ),
      ),
    onSuccess: (a) => {
      toast.success(`已任命 ${a.username ?? `#${a.user_id}`} 为${ROLE_LABEL[a.role]}`);
      form.reset({ user: null, role: form.getValues("role"), forumId: "" });
      void queryClient.invalidateQueries({ queryKey: keys.staff });
    },
  });
  const errors = form.formState.errors;
  return (
    <Card>
      <form
        className="grid gap-3 md:grid-cols-[2fr_1fr_1.5fr_auto] md:items-start"
        noValidate
        onSubmit={form.handleSubmit((v) => appoint.mutate(v))}
        aria-label="任命管理人员"
      >
        <Controller
          control={form.control}
          name="user"
          render={({ field }) => (
            <UserPicker
              label="用户"
              value={field.value}
              onChange={field.onChange}
              error={errors.user?.message}
            />
          )}
        />
        <Field label="角色">
          <select className={selectClass} {...form.register("role")}>
            <option value="moderator">版主（只管负责的板块）</option>
            <option value="admin">管理员（全站 + 仲裁）</option>
          </select>
        </Field>
        {role === "moderator" ? (
          <Field label="负责的板块" error={errors.forumId?.message}>
            <select className={selectClass} {...form.register("forumId")}>
              <option value="">选择板块</option>
              {(forums.data ?? []).map((f) => (
                <option key={f.id} value={f.id}>
                  {f.name}
                </option>
              ))}
            </select>
          </Field>
        ) : (
          <p className="text-sm text-fg-muted md:pt-7">管理员全站有效，并参与仲裁分配。</p>
        )}
        <Button type="submit" className="md:mt-6" disabled={appoint.isPending}>
          {appoint.isPending ? "提交中…" : "任命"}
        </Button>
      </form>
      {appoint.isError ? (
        <p role="alert" className="mt-2 text-sm text-danger">
          {errorMessage(appoint.error)}
        </p>
      ) : null}
    </Card>
  );
}
