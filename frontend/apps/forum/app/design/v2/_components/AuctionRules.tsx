import { Card } from "@akari/ui";
import { formatCnyPerUsdt } from "../../../../lib/fees";
import type { AuctionTerms } from "./AuctionPanel";

/** 拍卖规则（plan 第 7 节拍卖模式、ADR 0011）：出价不上链、成交证明、付款期、违约与递补。 */
export function AuctionRules({ terms: t }: { terms: AuctionTerms }) {
  const rules = [
    "出价在论坛内进行，不上链、不花 gas，也不用 Face ID；出价记录只追加、公开可查。",
    t.balancePercent > 0
      ? `出价资格：钱包余额不低于出价的 ${t.balancePercent}%。卖家本人不能出价。`
      : "不限出价资格。卖家本人不能出价。",
    `结束时达到保留价即成交：平台签发成交证明，赢家需在 ${t.payHours} 小时内一次 Face ID 付款建单。`,
    "付款后资金进入担保合约，发货、确认、争议与普通担保交易完全相同。",
    "赢家逾期未付记违约、禁拍 30 天，由次高出价者在 24 小时内递补。",
    ...(t.currency === "CNY" && t.cnyPerUsdt
      ? [
          `本场以人民币计价，按卖家签名的固定汇率 ${formatCnyPerUsdt(t.cnyPerUsdt)} 换算，付款时以 USDT 结算；平台无法改动汇率与保留价。`,
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
