import { cnyFenToUsdt } from "@akari/fees";
import { formatUsdt, parseUsdt } from "@akari/ui";
import { formatCnyFen, parseCnyFen } from "../../../../lib/fees";
import type { AuctionFacts } from "../_mock/data";

type Currency = AuctionFacts["currency"];

/** 拍卖币种金额（USDT 最小单位或人民币分）→「¥1,280.00」或「128.00 USDT」。 */
export function formatAuctionAmount(currency: Currency, amount: bigint): string {
  return currency === "CNY" ? formatCnyFen(amount) : `${formatUsdt(amount)} USDT`;
}

/** 填进输入框的写法：不带符号和千分位（「1300.00」）。 */
export function plainAuctionAmount(currency: Currency, amount: bigint): string {
  return currency === "CNY"
    ? `${amount / 100n}.${(amount % 100n).toString().padStart(2, "0")}`
    : formatUsdt(amount).replaceAll(",", "");
}

export function parseAuctionAmount(currency: Currency, input: string): bigint | null {
  return currency === "CNY" ? parseCnyFen(input) : parseUsdt(input);
}

/** 人民币拍卖价按卖家签名汇率折合的 USDT（最小单位，向上取整，与合约一致）。 */
export function auctionUsdt(fen: bigint, cnyPerUsdt: number): bigint {
  return cnyFenToUsdt(fen, BigInt(cnyPerUsdt));
}
