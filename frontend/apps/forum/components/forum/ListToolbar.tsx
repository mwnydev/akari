import type { ReactNode } from "react";

/** 列表顶部一行：左边排序标签页（与筛选），右边发帖按钮；整行底部一条分隔线，选中标签的下划线压在上面。 */
export function ListToolbar({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="mb-1 flex flex-wrap items-end gap-x-3 gap-y-2 border-b border-border">
      {children}
      {action ? <div className="mb-1.5 ml-auto flex items-center gap-2">{action}</div> : null}
    </div>
  );
}
