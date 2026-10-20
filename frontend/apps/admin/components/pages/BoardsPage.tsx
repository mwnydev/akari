"use client";

import { Badge, Button, LoadingState } from "@akari/ui";
import dynamic from "next/dynamic";
import { useState } from "react";
import { PALETTE } from "../../lib/boards";
import { useForums } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { Table, Td } from "../Table";

const BoardForm = dynamic(() => import("../forms/BoardForm"), {
  ssr: false,
  loading: () => <LoadingState />,
});

function Swatch({ hex }: { hex: string | null | undefined }) {
  const label = PALETTE.find((p) => p.hex === hex)?.label;
  return hex ? (
    <span className="inline-flex items-center gap-1.5">
      <span aria-hidden className="size-3 rounded-[3px]" style={{ backgroundColor: hex }} />
      {label ?? hex}
    </span>
  ) : (
    <span className="text-fg-muted">轮换</span>
  );
}

/** 板块管理：名称、简介、色块、排序与能否发交易帖、拍卖帖（站长；改这两个开关要再验证）。 */
export function BoardsPage() {
  const forums = useForums();
  const [editing, setEditing] = useState<number | null>(null);
  return (
    <Section
      title="全部板块"
      description="改动立即生效（主题页缓存里的板块名与色块最长 10 分钟后更新），每次修改写审计日志并推送管理员群。"
    >
      <QueryView query={forums}>
        {(boards) => {
          const board = boards.find((b) => b.id === editing);
          return (
            <>
              <Table label="板块" head={["排序", "名称", "简介", "色块", "交易帖", "拍卖帖", ""]}>
                {boards.map((b) => (
                  <tr key={b.id} data-testid="board-row">
                    <Td className="amount">{b.position}</Td>
                    <Td>
                      {b.parent_id != null ? <span className="text-fg-muted">└ </span> : null}
                      {b.name}
                      <span className="ml-1 text-xs text-fg-muted">/{b.slug}</span>
                    </Td>
                    <Td className="max-w-xs truncate">{b.description}</Td>
                    <Td>
                      <Swatch hex={b.color} />
                    </Td>
                    <Td>{b.allow_trade ? <Badge tone="success">允许</Badge> : "—"}</Td>
                    <Td>{b.allow_auction ? <Badge tone="success">允许</Badge> : "—"}</Td>
                    <Td>
                      <Button size="sm" variant="secondary" onClick={() => setEditing(b.id)}>
                        编辑
                      </Button>
                    </Td>
                  </tr>
                ))}
              </Table>
              {board ? (
                <BoardForm key={board.id} board={board} onDone={() => setEditing(null)} />
              ) : null}
            </>
          );
        }}
      </QueryView>
    </Section>
  );
}
