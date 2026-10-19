"use client";

import type { Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import { useState } from "react";
import { api, unwrap } from "../../lib/api";
import { formatTime } from "../../lib/format";
import { keys } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { useAdminSession } from "../session/SessionProvider";
import { EmptyRow, Table, Td } from "../Table";

const ReasonDialog = dynamic(() => import("../ReasonDialog").then((m) => m.ReasonDialog), {
  ssr: false,
});

type Suspension = Schemas["SuspensionView"];

const REASON: Record<string, string> = { anomaly: "异常检测自动暂停" };

/** 暂停详情（异常检测记下的窗口与计数）：服务端给的是 JSON，按「键 值」原样列出，React 负责转义。 */
function Detail({ detail }: { detail: Record<string, unknown> }) {
  const entries = Object.entries(detail);
  if (!entries.length) return <>—</>;
  return (
    <span className="text-xs text-fg-muted">
      {entries.map(([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`).join("；")}
    </span>
  );
}

export function SuspensionsPage() {
  const list = useQuery({
    queryKey: keys.suspensions,
    queryFn: () => unwrap(api.GET("/api/v1/admin/suspensions")),
  });
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const [target, setTarget] = useState<Suspension | null>(null);
  return (
    <Section
      title="被暂停管理权限的账号"
      description="短时间内删帖、封号等超过阈值的账号会被自动暂停论坛内的全部管理权限，核实后在这里恢复。"
    >
      <QueryView query={list}>
        {(data) => (
          <Table label="被暂停的账号" head={["账号", "原因", "详情", "暂停时间", ""]}>
            {data.suspensions.length === 0 ? <EmptyRow span={5}>没有被暂停的账号</EmptyRow> : null}
            {data.suspensions.map((s) => (
              <tr key={s.id}>
                <Td>{s.username ? `${s.username}（#${s.user_id}）` : `#${s.user_id}`}</Td>
                <Td>{REASON[s.reason] ?? s.reason}</Td>
                <Td>
                  <Detail detail={s.detail} />
                </Td>
                <Td className="whitespace-nowrap">{formatTime(s.created_at)}</Td>
                <Td className="text-right">
                  <Button size="sm" variant="ghost" onClick={() => setTarget(s)}>
                    恢复权限
                  </Button>
                </Td>
              </tr>
            ))}
          </Table>
        )}
      </QueryView>
      {target ? (
        <ReasonDialog
          title={`恢复 ${target.username ?? `#${target.user_id}`} 的管理权限`}
          description="确认这些操作是正常管理后再恢复；如是误操作或盗号，先去「批量撤销」处理。"
          confirmLabel="恢复"
          onClose={() => setTarget(null)}
          onConfirm={async (reason) => {
            await sensitive(() =>
              unwrap(
                api.POST("/api/v1/admin/suspensions/{id}/reinstate", {
                  params: { path: { id: target.id }, query: { reason } },
                }),
              ),
            );
            toast.success("已恢复管理权限");
            await queryClient.invalidateQueries({ queryKey: keys.suspensions });
          }}
        />
      ) : null}
    </Section>
  );
}
