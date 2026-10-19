import { Badge } from "@akari/ui";
import { Gavel, ShieldCheck } from "lucide-react";

/** 主题类型标签：担保交易 / 拍卖。描边小方标签，和置顶、精华等实底状态标签区分。 */
export function KindBadge({ kind, className }: { kind: "trade" | "auction"; className?: string }) {
  return (
    <Badge tone="outline" className={className}>
      {kind === "trade" ? (
        <ShieldCheck size={12} aria-hidden className="text-success" />
      ) : (
        <Gavel size={12} aria-hidden className="text-accent" />
      )}
      {kind === "trade" ? "担保交易" : "拍卖"}
    </Badge>
  );
}
