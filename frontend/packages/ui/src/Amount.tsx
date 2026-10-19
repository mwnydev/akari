import { formatUsdt } from "./format";

export interface AmountProps {
  /** USDT 最小单位（6 位小数），bigint 或十进制字符串 */
  value: bigint | string;
  className?: string;
}

export function Amount({ value, className }: AmountProps) {
  return (
    <span
      className={className ? `amount whitespace-nowrap ${className}` : "amount whitespace-nowrap"}
    >
      {formatUsdt(value)} <span className="text-[0.85em] text-fg-muted">USDT</span>
    </span>
  );
}
