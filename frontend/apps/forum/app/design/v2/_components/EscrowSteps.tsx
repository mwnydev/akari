import { ShieldCheck } from "lucide-react";

const STEPS = ["付款进担保合约", "卖家发货", "你确认收货", "合约放款给卖家"];

/** 担保说明：资金托管在合约里，确认收货后才放款；平台和管理员都转不走。 */
export function EscrowSteps({ confirmDays }: { confirmDays: number }) {
  return (
    <div className="rounded-lg bg-success-soft p-3 text-sm">
      <p className="flex items-center gap-1.5 font-medium text-success">
        <ShieldCheck size={16} aria-hidden />
        资金托管在合约里，确认收货后才放款
      </p>
      <ol className="mt-2 grid grid-cols-4 gap-1 text-center text-xs">
        {STEPS.map((s, i) => (
          <li key={s} className="flex flex-col items-center gap-1">
            <span className="amount flex size-6 items-center justify-center rounded-full bg-surface font-medium text-success">
              {i + 1}
            </span>
            <span className="leading-snug">{s}</span>
          </li>
        ))}
      </ol>
      <p className="mt-2 text-xs text-fg-muted">
        确认期 {confirmDays}{" "}
        天内可发起争议，由仲裁员裁决；期满未确认自动放款。平台和管理员都转不走托管资金。
      </p>
    </div>
  );
}
