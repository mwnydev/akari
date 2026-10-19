"use client";

import type { Schemas } from "@akari/api-client";
import { Badge, Button } from "@akari/ui";
import { SegmentedControl } from "@akari/ui/tabs";
import { useQuery } from "@tanstack/react-query";
import { Download } from "lucide-react";
import { useState } from "react";
import { api, unwrap } from "../../lib/api";
import { FORUM_ORIGIN } from "../../lib/config";
import { CASE_STATUS, formatTime, formatToken, userLabel } from "../../lib/format";
import { keys, useStaffNames } from "../../lib/queries";
import { batchFileName, onTimeRate, percent } from "../../lib/arbitration";
import { QueryView, Section } from "../Section";
import { EmptyRow, Table, Td } from "../Table";

const RANGES = [
  { value: "7", label: "7 天" },
  { value: "30", label: "30 天" },
  { value: "90", label: "90 天" },
] as const;

function caseHref(id: number): string {
  return `${FORUM_ORIGIN}/arbitration/${id}`;
}

function Flags({ c }: { c: Schemas["AttentionCase"] }) {
  return (
    <span className="flex flex-wrap gap-1">
      <Badge tone={c.status === "halted" || c.status === "owner_review" ? "warning" : "neutral"}>
        {CASE_STATUS[c.status] ?? c.status}
      </Badge>
      {c.unstaffed ? <Badge tone="danger">缺人：需站长处理</Badge> : null}
      {c.timeout_soon ? <Badge tone="danger">临近超时兜底</Badge> : null}
      {c.panel_size === 2 ? <Badge tone="outline">双人</Badge> : null}
    </span>
  );
}

/** 下载给仲裁 Safe 签名人导入「Transaction Builder」的 JSON 文件。 */
function download(batch: Schemas["ExecutionBatch"]) {
  const blob = new Blob([JSON.stringify(batch.transaction_builder, null, 2)], {
    type: "application/json",
  });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = batchFileName(batch.chain_id, new Date());
  a.click();
  URL.revokeObjectURL(url);
}

function Execution() {
  const batches = useQuery({
    queryKey: keys.execution,
    queryFn: () => unwrap(api.GET("/api/v1/admin/arbitration/execution")),
  });
  return (
    <Section
      title="待仲裁 Safe 执行"
      description="申诉期已过（或复核已定）的裁决，每天合并成一笔。签名人在 Safe 官方界面导入文件后各自签名（3 人 2/3），合约只能把钱分给买卖双方。"
    >
      <QueryView query={batches}>
        {(data) =>
          data.length === 0 ? (
            <p className="text-sm text-fg-muted">现在没有待执行的裁决。</p>
          ) : (
            <div className="space-y-3">
              {data.map((b) => (
                <div key={b.chain_id} className="space-y-2">
                  <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
                    <span>
                      链 {b.chain_id} · {b.cases.length} 件 · Safe{" "}
                      <span className="amount">{b.safe ?? "未配置（签名人在界面里选择）"}</span>
                    </span>
                    <Button size="sm" onClick={() => download(b)}>
                      <Download size={16} aria-hidden />
                      下载 Safe 批量文件
                    </Button>
                  </div>
                  <Table
                    label={`链 ${b.chain_id} 待执行`}
                    head={["案件", "链上订单", "本金判给买家", "保证金赔给买家", "合约"]}
                  >
                    {b.cases.map((c) => (
                      <tr key={c.case_id}>
                        <Td>
                          <a className="text-accent hover:underline" href={caseHref(c.case_id)}>
                            #{c.case_id}
                          </a>
                        </Td>
                        <Td className="amount">{c.chain_order_id}</Td>
                        <Td className="amount">{percent(c.to_buyer_bps)}</Td>
                        <Td className="amount">{percent(c.bond_to_buyer_bps)}</Td>
                        <Td className="amount text-xs">{c.to}</Td>
                      </tr>
                    ))}
                  </Table>
                </div>
              ))}
            </div>
          )
        }
      </QueryView>
    </Section>
  );
}

/** 仲裁总览：工作量与按时率（计酬只看这些，绝不看裁决方向）、需要站长处理的案件、Safe 批量执行文件。 */
export function ArbitrationPage() {
  const [days, setDays] = useState<(typeof RANGES)[number]["value"]>("30");
  const names = useStaffNames();
  const overview = useQuery({
    queryKey: keys.arbitration(Number(days)),
    queryFn: () =>
      unwrap(api.GET("/api/v1/admin/arbitration", { params: { query: { days: Number(days) } } })),
  });
  return (
    <>
      <Section
        title="需要站长处理或关注"
        description="待终裁、申诉复核、已叫停、待执行、找不到人接案与临近 resolveTimeout 兜底的案件。裁决与叫停在论坛的仲裁页面用裁决 Passkey 完成。"
      >
        <QueryView query={overview}>
          {(data) => (
            <Table
              label="需要关注的案件"
              head={["案件", "状态", "金额", "举证截止", "申诉截止", "超时兜底", "叫停理由"]}
            >
              {data.cases.length === 0 ? <EmptyRow span={7}>没有需要处理的案件</EmptyRow> : null}
              {data.cases.map((c) => (
                <tr key={c.case_id} data-testid="attention-row">
                  <Td>
                    <a className="text-accent hover:underline" href={caseHref(c.case_id)}>
                      #{c.case_id}
                    </a>
                  </Td>
                  <Td>
                    <Flags c={c} />
                  </Td>
                  <Td className="amount whitespace-nowrap">
                    {formatToken(c.amount, c.token_decimals)}
                  </Td>
                  <Td className="whitespace-nowrap">{formatTime(c.evidence_until)}</Td>
                  <Td className="whitespace-nowrap">{formatTime(c.appeal_until)}</Td>
                  <Td className="whitespace-nowrap">{formatTime(c.timeout_at)}</Td>
                  <Td>{c.halt_reason ?? "—"}</Td>
                </tr>
              ))}
            </Table>
          )}
        </QueryView>
      </Section>
      <Section
        title="仲裁员工作量"
        description="按分配时间统计；按时率 = 期限内提交 ÷ 已提交。"
        actions={
          <SegmentedControl
            label="统计范围"
            options={[...RANGES]}
            value={days}
            onValueChange={setDays}
          />
        }
      >
        <QueryView query={overview}>
          {(data) => (
            <Table
              label="仲裁员工作量"
              head={["仲裁员", "分配", "已裁决", "按时率", "超时改派", "在岗", "站长终裁 / 复核"]}
            >
              {data.workload.arbitrators.length === 0 ? (
                <EmptyRow span={7}>这段时间没有分配</EmptyRow>
              ) : null}
              {data.workload.arbitrators.map((w) => (
                <tr key={w.arbitrator_id}>
                  <Td>{userLabel(w.arbitrator_id, names)}</Td>
                  <Td className="amount">{w.assigned}</Td>
                  <Td className="amount">{w.ruled}</Td>
                  <Td className="amount">{onTimeRate(w.on_time, w.ruled)}</Td>
                  <Td className="amount">{w.reassigned}</Td>
                  <Td className="amount">{w.open}</Td>
                  <Td className="amount">{w.owner_rulings}</Td>
                </tr>
              ))}
            </Table>
          )}
        </QueryView>
      </Section>
      <Execution />
    </>
  );
}
