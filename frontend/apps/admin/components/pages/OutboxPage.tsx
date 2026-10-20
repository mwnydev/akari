"use client";

import { errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, unwrap } from "../../lib/api";
import { formatTime } from "../../lib/format";
import { keys } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { EmptyRow, Table, Td } from "../Table";

type Action = "requeue" | "discard";

const DONE: Record<Action, string> = { requeue: "已重新投递", discard: "已丢弃" };

/**
 * 发件箱：每种消息的待发与死信条数，死信列表（每页 50 条，游标翻页）。
 * 死信是重试也不会成功（违反约束、内容不对）或重试用尽的消息；修好原因后重新投递，或丢弃。
 */
export function OutboxPage() {
  const queryClient = useQueryClient();
  const [cursors, setCursors] = useState<(number | null)[]>([null]);
  const cursor = cursors[cursors.length - 1] ?? null;
  const page = useQuery({
    queryKey: keys.outbox(cursor),
    queryFn: () => unwrap(api.GET("/api/v1/admin/outbox", { params: { query: { cursor } } })),
  });
  const act = useMutation({
    mutationFn: ({ action, ids }: { action: Action; ids: number[] }) =>
      unwrap(
        action === "requeue"
          ? api.POST("/api/v1/admin/outbox/requeue", { body: { ids } })
          : api.POST("/api/v1/admin/outbox/discard", { body: { ids } }),
      ),
    onSuccess: async (res, { action }) => {
      toast.success(`${DONE[action]} ${res.ids.length} 条`);
      await queryClient.invalidateQueries({ queryKey: [keys.outbox(null)[0]] });
    },
    onError: (err) => toast.error(errorMessage(err)),
  });

  return (
    <Section
      title="发件箱"
      description="站内通知、邮件与管理员群告警都经发件箱投递。重试也不会成功或重试用尽的消息进死信（14 天后自动丢弃）；修好原因后重新投递，或丢弃。每次处理写审计日志。"
    >
      <QueryView query={page}>
        {(data) => (
          <>
            <Table label="消息类型" head={["类型", "待发", "重试中", "死信", "最早待发"]}>
              {data.kinds.length === 0 ? <EmptyRow span={5}>没有待发消息和死信</EmptyRow> : null}
              {data.kinds.map((k) => (
                <tr key={k.kind}>
                  <Td className="amount">{k.kind}</Td>
                  <Td className="amount">{k.pending}</Td>
                  <Td className="amount">{k.retrying}</Td>
                  <Td className="amount">{k.dead}</Td>
                  <Td className="whitespace-nowrap">
                    {k.oldest_pending ? formatTime(k.oldest_pending) : "—"}
                  </Td>
                </tr>
              ))}
            </Table>
            <Table label="死信" head={["#", "类型", "次数", "最后的错误", "创建", "进入死信", ""]}>
              {data.dead.length === 0 ? <EmptyRow span={7}>没有死信</EmptyRow> : null}
              {data.dead.map((d) => (
                <tr key={d.id} data-testid="dead-letter-row">
                  <Td className="amount text-fg-muted">{d.id}</Td>
                  <Td className="amount">{d.kind}</Td>
                  <Td className="amount">{d.attempts}</Td>
                  <Td className="max-w-80">
                    <code className="text-xs break-all text-fg-muted">{d.last_error ?? "—"}</code>
                  </Td>
                  <Td className="whitespace-nowrap">{formatTime(d.created_at)}</Td>
                  <Td className="whitespace-nowrap">{formatTime(d.failed_at)}</Td>
                  <Td className="whitespace-nowrap text-right">
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={act.isPending}
                      onClick={() => act.mutate({ action: "requeue", ids: [d.id] })}
                    >
                      重新投递
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={act.isPending}
                      onClick={() => act.mutate({ action: "discard", ids: [d.id] })}
                    >
                      丢弃
                    </Button>
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
