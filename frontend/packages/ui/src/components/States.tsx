import { CircleAlert, Inbox } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../cn";

/** 空 / 加载 / 出错三件套：每个列表和异步区域都必须处理这三种状态。 */
export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center gap-2 px-4 py-10 text-center">
      <Inbox size={32} className="text-fg-muted" aria-hidden />
      <p className="font-medium">{title}</p>
      {description ? <p className="max-w-xs text-sm text-fg-muted">{description}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}

export function LoadingState({ rows = 3, label = "加载中" }: { rows?: number; label?: string }) {
  return (
    <div role="status" aria-label={label} className="space-y-3 py-2">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex animate-pulse-soft items-center gap-3">
          <div className="size-10 shrink-0 rounded-full bg-surface-2" />
          <div className="flex-1 space-y-2">
            <div className={cn("h-3 rounded-pill bg-surface-2", i % 2 ? "w-1/2" : "w-3/4")} />
            <div className="h-3 w-1/3 rounded-pill bg-surface-2" />
          </div>
        </div>
      ))}
    </div>
  );
}

export function ErrorState({
  title = "出错了",
  message,
  action,
}: {
  title?: string;
  /** 已翻译成人话的错误说明，不显示错误码原文 */
  message: string;
  action?: ReactNode;
}) {
  return (
    <div role="alert" className="flex flex-col items-center gap-2 px-4 py-10 text-center">
      <CircleAlert size={32} className="text-danger" aria-hidden />
      <p className="font-medium">{title}</p>
      <p className="max-w-xs text-sm text-fg-muted">{message}</p>
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}
