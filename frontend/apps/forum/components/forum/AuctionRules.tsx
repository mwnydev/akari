import { Card, formatUsdt } from "@akari/ui";
import { days, hours, type AuctionEligibility } from "../../lib/auction";
import { formatCnyPerUsdt } from "../../lib/fees";
import type { AuctionPanelData } from "./AuctionPanel";

function eligibilityText(e: AuctionEligibility): string {
  switch (e.kind) {
    case "none":
      return "不限出价资格";
    case "percent":
      return `出价资格：钱包余额不低于出价的 ${e.percent}%`;
    case "fixed":
      return `出价资格：钱包余额不低于 ${formatUsdt(BigInt(e.usdt))} USDT`;
  }
}

/** 拍卖规则（plan 第 7 节拍卖模式、ADR 0011）：出价不上链、成交证明、付款期、违约与递补。 */
export function AuctionRules({
  auction: a,
}: {
  auction: Pick<
    AuctionPanelData,
    | "currency"
    | "cnyPerUsdt"
    | "eligibility"
    | "payWindowSecs"
    | "runnerUpWindowSecs"
    | "defaultBanSecs"
  >;
}) {
  const rules = [
    "出价在论坛内进行，不上链、不花 gas，也不用 Face ID；出价记录只追加、公开可查。",
    `${eligibilityText(a.eligibility)}；出价时校验，结束时再校验一次赢家。卖家本人不能出价。`,
    `结束时达到保留价即成交：平台签发成交证明，赢家需在 ${hours(a.payWindowSecs)} 小时内一次 Face ID 付款建单。`,
    "付款后资金进入担保合约，发货、确认、争议与普通担保交易完全相同。",
    `赢家逾期未付记违约、禁拍 ${days(a.defaultBanSecs)} 天，由次高出价者在 ${hours(a.runnerUpWindowSecs)} 小时内递补。`,
    ...(a.currency === "CNY" && a.cnyPerUsdt !== null
      ? [
          `本场以人民币计价，按卖家签名的固定汇率 ${formatCnyPerUsdt(a.cnyPerUsdt)} 换算，付款时以 USDT 结算；平台无法改动汇率与保留价。`,
        ]
      : []),
  ];
  return (
    <Card>
      <h2 className="text-sm font-medium">拍卖规则</h2>
      <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-fg-muted">
        {rules.map((r) => (
          <li key={r}>{r}</li>
        ))}
      </ul>
    </Card>
  );
}
