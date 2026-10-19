"use client";

import type { Schemas } from "@akari/api-client";
import { Badge, Button } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, unwrap } from "../../lib/api";
import { actionLabel, formatTime, userLabel } from "../../lib/format";
import { keys, useStaffNames } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { EmptyRow, Table, Td } from "../Table";
import { UserPicker, type PickedUser } from "../UserPicker";

type Entry = Schemas["AuditEntryView"];

function Status({ e }: { e: Entry }) {
  if (e.reverts != null) return <Badge tone="accent">撤销了 #{e.reverts}</Badge>;
  if (e.reverted) return <Badge tone="warning">已被撤销</Badge>;
  return null;
}

/** 只增审计日志（新的在前，每页 50 条，游标翻页）。详情是 JSON 文本，React 转义后显示。 */
export function AuditPage() {
  const names = useStaffNames();
  const [actor, setActor] = useState<PickedUser | null>(null);
  // 游标栈：栈顶是当前页的游标（第一页为 null），「上一页」出栈
  const [cursors, setCursors] = useState<(number | null)[]>([null]);
  const cursor = cursors[cursors.length - 1] ?? null;
  const actorId = actor?.id ?? null;
  const page = useQuery({
    queryKey: keys.audit(actorId, cursor),
    queryFn: () =>
      unwrap(
        api.GET("/api/v1/admin/audit", {
          params: { query: { actor_id: actorId, cursor } },
        }),
      ),
  });

  return (
    <Section
      title="审计日志"
      description="所有管理操作与后台改动都只增不改；撤销也是新的一条记录，指向被撤销的那条。"
      actions={
        <div className="w-72">
          <UserPicker
            label="只看这个账号的操作"
            value={actor}
            onChange={(u) => {
              setActor(u);
              setCursors([null]);
            }}
          />
        </div>
      }
    >
      <QueryView query={page}>
        {(data) => (
          <>
            <Table
              label="审计日志"
              head={["#", "时间", "操作人", "操作", "对象", "理由", "详情", ""]}
            >
              {data.entries.length === 0 ? <EmptyRow span={8}>没有记录</EmptyRow> : null}
              {data.entries.map((e) => (
                <tr key={e.id} data-testid="audit-row">
                  <Td className="amount text-fg-muted">{e.id}</Td>
                  <Td className="whitespace-nowrap">{formatTime(e.created_at)}</Td>
                  <Td>{e.actor_id === 0 ? "系统" : userLabel(e.actor_id, names)}</Td>
                  <Td className="whitespace-nowrap">{actionLabel(e.action)}</Td>
                  <Td className="amount">{e.target ?? "—"}</Td>
                  <Td className="max-w-56">{e.reason ?? "—"}</Td>
                  <Td className="max-w-72">
                    <code className="text-xs break-all text-fg-muted">
                      {JSON.stringify(e.detail)}
                    </code>
                  </Td>
                  <Td>
                    <Status e={e} />
                  </Td>
                </tr>
              ))}
            </Table>
            <div className="flex justify-between gap-2">
              <Button
                variant="secondary"
                size="sm"
                disabled={cursors.length <= 1}
                onClick={() => setCursors((c) => c.slice(0, -1))}
              >
                上一页
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={data.next_cursor == null}
                onClick={() => setCursors((c) => [...c, data.next_cursor ?? null])}
              >
                下一页
              </Button>
            </div>
          </>
        )}
      </QueryView>
    </Section>
  );
}
