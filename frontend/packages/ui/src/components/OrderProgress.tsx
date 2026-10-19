import { Check } from "lucide-react";
import { Amount } from "../Amount";
import { cn } from "../cn";
import { Badge, type BadgeTone } from "./Badge";

/** 链上订单状态（plan 第 6 节）；没有「已创建未付款」状态 */
export type OnceState = "funded" | "delivered" | "released" | "refunded" | "settled" | "disputed";

const outcome: Partial<Record<OnceState, { label: string; tone: BadgeTone }>> = {
  refunded: { label: "已退款", tone: "neutral" },
  settled: { label: "已协商和解", tone: "neutral" },
  disputed: { label: "争议中", tone: "warning" },
};

export function OnceProgress({
  state,
  delivered = state === "delivered" || state === "released",
  hint,
}: {
  state: OnceState;
  /** 退款、和解、争议时卖家是否已标记发货 */
  delivered?: boolean;
  hint?: string;
}) {
  const steps = ["已付款", "已发货", "已放款"];
  const reached = state === "released" ? 3 : delivered ? 2 : 1;
  const end = outcome[state];
  return (
    <div>
      <ol className="flex items-center" aria-label="订单进度">
        {steps.map((s, i) => {
          const done = i < reached;
          return (
            <li key={s} className={cn("flex items-center", i < steps.length - 1 && "flex-1")}>
              <span className="flex flex-col items-center gap-1">
                <span
                  className={cn(
                    "flex size-7 items-center justify-center rounded-full text-sm font-medium",
                    done ? "bg-accent-fill text-accent-fg" : "bg-surface-2 text-fg-muted",
                  )}
                >
                  {done ? <Check size={16} aria-hidden /> : i + 1}
                </span>
                <span
                  className={cn(
                    "text-xs whitespace-nowrap",
                    done ? "font-medium" : "text-fg-muted",
                  )}
                >
                  {s}
                </span>
              </span>
              {i < steps.length - 1 ? (
                <span
                  className={cn(
                    "mx-1 mb-5 h-0.5 flex-1 rounded-pill",
                    i + 1 < reached ? "bg-accent" : "bg-surface-2",
                  )}
                />
              ) : null}
            </li>
          );
        })}
      </ol>
      {end || hint ? (
        <p className="mt-3 flex flex-wrap items-center gap-2 text-sm text-fg-muted">
          {end ? <Badge tone={end.tone}>{end.label}</Badge> : null}
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export interface StreamProgressProps {
  periods: number;
  elapsed: number;
  /** 以下金额都是最小单位字符串 */
  unlocked: string;
  claimed: string;
  net: string;
  periodLabel: string;
  disputed?: boolean;
}

/** 分期进度：总期数、已过期数、已解锁、已领取、剩余可退（plan 第 7 节）。 */
export function StreamProgress({
  periods,
  elapsed,
  unlocked,
  claimed,
  net,
  periodLabel,
  disputed = false,
}: StreamProgressProps) {
  const shown = Math.min(elapsed, periods);
  const remaining = (BigInt(net) - BigInt(unlocked)).toString();
  return (
    <div>
      <div className="mb-2 flex items-baseline justify-between text-sm">
        <span>
          {periodLabel} · 第 <span className="amount font-medium">{shown}</span> / {periods} 期
        </span>
        {disputed ? <Badge tone="warning">争议中 · 解锁已冻结</Badge> : null}
      </div>
      <div
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={periods}
        aria-valuenow={shown}
        className="flex h-2.5 gap-px overflow-hidden rounded-pill bg-surface-2"
      >
        <div
          className={cn("h-full rounded-pill", disputed ? "bg-warning" : "bg-accent")}
          style={{ width: `${(shown / periods) * 100}%` }}
        />
      </div>
      <dl className="mt-3 grid grid-cols-3 gap-2 text-sm">
        <div>
          <dt className="text-fg-muted">已解锁</dt>
          <dd>
            <Amount value={unlocked} />
          </dd>
        </div>
        <div>
          <dt className="text-fg-muted">已领取</dt>
          <dd>
            <Amount value={claimed} />
          </dd>
        </div>
        <div>
          <dt className="text-fg-muted">剩余可退</dt>
          <dd>
            <Amount value={remaining} />
          </dd>
        </div>
      </dl>
    </div>
  );
}
