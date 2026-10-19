import { Button } from "@akari/ui";
import { Plus } from "lucide-react";
import type { ReactNode } from "react";

/** 列表顶部一行：左边排序标签页（与筛选），右边实底的发帖按钮；整行底部一条分隔线，选中标签的下划线压在上面。 */
export function ListToolbar({ children, newLabel }: { children: ReactNode; newLabel: string }) {
  return (
    <div className="mb-1 flex flex-wrap items-end gap-x-3 gap-y-2 border-b border-border">
      {children}
      <Button size="sm" className="mb-1.5 ml-auto">
        <Plus size={16} aria-hidden />
        {newLabel}
      </Button>
    </div>
  );
}
