import { errorMessage } from "@akari/api-client";
import { Button, ErrorState, LoadingState } from "@akari/ui";
import type { UseQueryResult } from "@tanstack/react-query";
import type { ReactNode } from "react";

/** 一块异步数据：加载中、出错（含 403）与有数据三种状态。 */
export function QueryView<T>({
  query,
  children,
}: {
  query: UseQueryResult<T>;
  children: (data: T) => ReactNode;
}) {
  if (query.isPending) return <LoadingState />;
  if (query.isError) {
    return (
      <ErrorState
        message={errorMessage(query.error)}
        action={
          <Button variant="secondary" onClick={() => void query.refetch()}>
            重试
          </Button>
        }
      />
    );
  }
  return <>{children(query.data)}</>;
}

/** 页面里的一个区块：标题、说明与右侧操作。 */
export function Section({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="space-y-2">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div>
          <h2 className="text-lg font-semibold">{title}</h2>
          {description ? <p className="text-sm text-fg-muted">{description}</p> : null}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}
