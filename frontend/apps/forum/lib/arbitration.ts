// 仲裁案件的状态文案与时间线（plan 第 7 节：举证 72 小时 → 裁决 48 小时 → 公示与申诉 72 小时 → Safe 执行）。

import type { Schemas } from "@akari/api-client";

type Case = Schemas["CaseDetail"];

export const CASE_STATUS: Record<string, string> = {
  reviewing: "审理中",
  owner_review: "待站长终裁",
  appeal_window: "已公示 · 申诉期",
  appealed: "申诉复核中",
  ready: "待仲裁 Safe 执行",
  halted: "已叫停",
};

export const CLOSE_REASON: Record<string, string> = {
  executed: "已由仲裁 Safe 执行",
  settled: "已和解或超时兜底结算",
  reverted: "争议被区块回滚撤销",
};

export const RULING_KIND: Record<Schemas["RulingKind"], string> = {
  panel: "仲裁员裁决",
  owner: "站长终裁",
  appeal: "申诉复核",
};

export const SEAT_LABEL: Record<string, string> = {
  buyer: "买家",
  seller: "卖家",
  panel: "本案仲裁员",
  appeal: "申诉复核人",
  owner: "站长",
};

/** 状态文案；结案原因（与结案时间同时写入、重新打开时同时清空）优先。 */
export function caseStatusLabel(c: { status: string; close_reason?: string | null }): string {
  if (c.close_reason) return CLOSE_REASON[c.close_reason] ?? "已结案";
  return CASE_STATUS[c.status] ?? c.status;
}

export interface TimelineStep {
  label: string;
  /** 已发生或计划中的时间；不知道具体时间时为空 */
  at: string | null;
  done: boolean;
}

/**
 * 案件时间线：已发生的按时间排序，接着是还没到的期限。只用访问者能看到的字段——
 * 两人盲审期间仲裁员只看得到自己的裁决，当事人在公示前看不到任何裁决。
 */
export function caseTimeline(c: Case, now: Date): TimelineStep[] {
  const past = (iso: string | null | undefined) => iso != null && new Date(iso) <= now;
  const steps: TimelineStep[] = [];
  const disputedAt = c.order.dispute?.disputed_at ?? null;
  if (disputedAt) steps.push({ label: "发起争议", at: disputedAt, done: true });
  steps.push({
    label: past(c.evidence_until) ? "举证期结束" : "举证期截止",
    at: c.evidence_until,
    done: past(c.evidence_until),
  });
  for (const a of c.assignments) {
    steps.push({
      label: a.role === "appeal" ? "分配申诉复核人" : "分配仲裁员",
      at: a.assigned_at,
      done: true,
    });
    if (a.ended_reason === "reassigned") {
      steps.push({ label: "超时未裁决，已改派", at: a.due_at, done: true });
    }
  }
  for (const r of c.rulings) {
    steps.push({ label: `提交${RULING_KIND[r.kind]}`, at: r.created_at, done: true });
  }
  if (c.published_at) steps.push({ label: "裁决公示", at: c.published_at, done: true });
  if (c.appeal_by != null) steps.push({ label: "当事人申诉", at: null, done: true });
  else if (c.appeal_until && !c.closed_at) {
    steps.push({
      label: past(c.appeal_until) ? "申诉期结束" : "申诉期截止",
      at: c.appeal_until,
      done: past(c.appeal_until),
    });
  }
  if (c.status === "halted" && !c.closed_at)
    steps.push({ label: "站长叫停", at: null, done: true });
  if (c.closed_at) {
    steps.push({ label: caseStatusLabel(c), at: c.closed_at, done: true });
  } else if (c.timeout) {
    steps.push({ label: "resolveTimeout 兜底可调用", at: c.timeout.at, done: past(c.timeout.at) });
  }
  // 已发生的在前（按时间，未知时间的保持原位），未到的期限在后
  const timed = (s: TimelineStep) => (s.at ? new Date(s.at).getTime() : Number.NaN);
  return steps
    .map((s, i) => ({ s, i }))
    .sort((a, b) => {
      if (a.s.done !== b.s.done) return a.s.done ? -1 : 1;
      const ta = timed(a.s);
      const tb = timed(b.s);
      return Number.isNaN(ta) || Number.isNaN(tb) || ta === tb ? a.i - b.i : ta - tb;
    })
    .map(({ s }) => s);
}

/** 只放行 http(s) 地址：聊天与证据里的图片地址不让 `javascript:` 之类的进 href / src。 */
export function httpUrl(raw: string | null | undefined): string | null {
  if (!raw) return null;
  try {
    const u = new URL(raw);
    return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
  } catch {
    return null;
  }
}
