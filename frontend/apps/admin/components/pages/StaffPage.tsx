"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Badge, Button, Card, Field } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import { useState } from "react";
import { Controller, useForm } from "react-hook-form";
import * as z from "zod/mini";
import { api, unwrap } from "../../lib/api";
import { formatTime, ROLE_LABEL, userLabel } from "../../lib/format";
import { keys, useForumNames, useForums, useStaff, useStaffNames } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { useAdminSession } from "../session/SessionProvider";
import { EmptyRow, Table, Td } from "../Table";
import { UserPicker } from "../UserPicker";

const ReasonDialog = dynamic(() => import("../ReasonDialog").then((m) => m.ReasonDialog), {
  ssr: false,
});

type Assignment = Schemas["StaffAssignment"];

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

function AppointForm() {
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

export function StaffPage() {
  const staff = useStaff();
  const names = useStaffNames();
  const forumNames = useForumNames();
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const [removing, setRemoving] = useState<Assignment | null>(null);

  return (
    <>
      <Section
        title="任命"
        description="任命与免职都要在 5 分钟内用后台 Passkey 再验证过，写入审计日志并推送管理员群。"
      >
        <AppointForm />
      </Section>
      <Section title="在任管理人员">
        <QueryView query={staff}>
          {(data) => (
            <Table label="在任管理人员" head={["用户", "角色", "范围", "任命时间", "任命人", ""]}>
              {data.staff.length === 0 ? <EmptyRow span={6}>还没有任命任何人</EmptyRow> : null}
              {data.staff.map((a) => (
                <tr key={a.id} data-testid="staff-row">
                  <Td>{a.username ? `${a.username}（#${a.user_id}）` : `#${a.user_id}`}</Td>
                  <Td>
                    <Badge tone={a.role === "moderator" ? "neutral" : "accent"}>
                      {ROLE_LABEL[a.role]}
                    </Badge>
                  </Td>
                  <Td>
                    {a.forum_id == null
                      ? "全站"
                      : (forumNames.get(a.forum_id) ?? `板块 #${a.forum_id}`)}
                  </Td>
                  <Td className="whitespace-nowrap">{formatTime(a.created_at)}</Td>
                  <Td>{a.granted_by == null ? "—" : userLabel(a.granted_by, names)}</Td>
                  <Td className="text-right">
                    {a.role === "owner" ? null : (
                      <Button size="sm" variant="ghost" onClick={() => setRemoving(a)}>
                        免去
                      </Button>
                    )}
                  </Td>
                </tr>
              ))}
            </Table>
          )}
        </QueryView>
      </Section>
      {removing ? (
        <ReasonDialog
          title={`免去 ${removing.username ?? `#${removing.user_id}`} 的${ROLE_LABEL[removing.role]}`}
          description="立即失效；之后由这个人的管理操作仍保留在审计日志里，可另行批量撤销。"
          confirmLabel="免去"
          danger
          onClose={() => setRemoving(null)}
          onConfirm={async (reason) => {
            await sensitive(() =>
              unwrap(
                api.DELETE("/api/v1/admin/staff/{id}", {
                  params: { path: { id: removing.id }, query: { reason } },
                }),
              ),
            );
            toast.success("已免去任命");
            await queryClient.invalidateQueries({ queryKey: keys.staff });
          }}
        />
      ) : null}
    </>
  );
}
