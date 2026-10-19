"use client";

import type { Schemas } from "@akari/api-client";
import { Badge, Button, LoadingState } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { useQueryClient } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import { useState } from "react";
import { api, unwrap } from "../../lib/api";
import { formatTime, ROLE_LABEL, userLabel } from "../../lib/format";
import { keys, useForumNames, useStaff, useStaffNames } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { useAdminSession } from "../session/SessionProvider";
import { EmptyRow, Table, Td } from "../Table";

const AppointForm = dynamic(() => import("../forms/AppointForm"), {
  ssr: false,
  loading: () => <LoadingState />,
});
const ReasonDialog = dynamic(() => import("../ReasonDialog").then((m) => m.ReasonDialog), {
  ssr: false,
});

type Assignment = Schemas["StaffAssignment"];

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
