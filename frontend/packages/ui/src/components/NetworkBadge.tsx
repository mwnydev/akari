import { cn } from "../cn";

/**
 * 网络标识：本站只用 Arbitrum One，动钱的页面永远显示（plan 第 5、14 节）。
 * `label` 只给开发 / 测试网用（如「本地测试链」），确认页必须显示真实签名所在的网络。
 */
export function NetworkBadge({
  className,
  label = "Arbitrum One",
}: {
  className?: string;
  label?: string | undefined;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-pill border border-border bg-surface px-2.5 py-0.5 text-sm font-medium",
        className,
      )}
    >
      <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden>
        <circle cx="8" cy="8" r="7.5" fill="#213147" />
        <path d="M8 3.2 4.2 9.8h1.7L8 6.1l2.1 3.7h1.7z" fill="#12AAFF" />
        <path d="m5.2 11.4.9-1.6h3.8l.9 1.6z" fill="#fff" />
      </svg>
      {label}
    </span>
  );
}
