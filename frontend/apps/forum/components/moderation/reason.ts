import * as z from "zod/mini";

/** 预设理由（plan 第 9 节），可再补充说明；合起来写进审计日志并通知对方。 */
export const PRESET_REASONS = [
  "违反版规",
  "广告或引流",
  "内容重复",
  "人身攻击",
  "交易欺诈",
  "优质内容",
  "误操作恢复",
  "其他",
] as const;

export interface ReasonValues {
  preset: string;
  note: string;
}

export const reasonFields = {
  preset: z.string().check(z.minLength(1, { error: "请选择理由" })),
  note: z.string().check(z.maxLength(400, { error: "补充说明最多 400 字" })),
};

/** 「其他」必须写补充说明，否则日志里看不出原因。 */
export function reasonComplete(v: ReasonValues): boolean {
  return v.preset !== "其他" || v.note.trim().length > 0;
}

export function composeReason(v: ReasonValues): string {
  const note = v.note.trim();
  return note ? `${v.preset}：${note}` : v.preset;
}
