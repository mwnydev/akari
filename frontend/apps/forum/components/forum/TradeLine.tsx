import { Amount, Badge, successRateLabel, type TradeReputation } from "@akari/ui";
import { Zap } from "lucide-react";
import { cnyReference } from "../../lib/fees";

/** 交易帖在列表里的摘要；接口没给的项不显示。 */
export interface TradeFacts {
  /** USDT 最小单位 */
  price: string;
  /** 卖家参考汇率（4 位小数定点数） */
  cnyPerUsdt?: number | null | undefined;
  instant?: boolean | undefined;
  stock?: number | null | undefined;
  seller?: TradeReputation | undefined;
}

/** 主题列表行里交易帖的信息：标价 ≈ 人民币、卖家成功率、发货方式、库存。 */
export function TradeLine({ trade }: { trade: TradeFacts }) {
  return (
    <>
      <span className="amount">
        <Amount value={trade.price} className="font-medium text-fg" />
        {trade.cnyPerUsdt ? ` ≈ ${cnyReference(trade.price, trade.cnyPerUsdt)}` : null}
      </span>
      {trade.seller ? <span className="amount">{successRateLabel(trade.seller)}</span> : null}
      {trade.instant === true ? (
        <Badge tone="success">
          <Zap size={12} aria-hidden />
          自动发卡
        </Badge>
      ) : trade.instant === false ? (
        <Badge>人工发货</Badge>
      ) : null}
      {trade.stock != null ? (
        <span className="amount">{trade.stock > 1 ? `库存 ${trade.stock}` : "仅 1 件"}</span>
      ) : null}
    </>
  );
}
