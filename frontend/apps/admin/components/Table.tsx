import { cn } from "@akari/ui";
import type { ReactNode } from "react";

/** 后台的紧凑表格：小字号、细分隔线、窄屏横向滚动。 */
export function Table({
  head,
  children,
  label,
}: {
  head: ReactNode[];
  children: ReactNode;
  label: string;
}) {
  return (
    <div className="overflow-x-auto rounded-md border border-border bg-surface">
      <table aria-label={label} className="w-full text-left text-sm">
        <thead className="bg-surface-2 text-xs text-fg-muted">
          <tr>
            {head.map((h, i) => (
              <th key={i} scope="col" className="px-3 py-2 font-medium whitespace-nowrap">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-border">{children}</tbody>
      </table>
    </div>
  );
}

export function Td({ children, className }: { children?: ReactNode; className?: string }) {
  return <td className={cn("px-3 py-1.5 align-top", className)}>{children}</td>;
}

/** 表格为空时占满一行的说明。 */
export function EmptyRow({ span, children }: { span: number; children: ReactNode }) {
  return (
    <tr>
      <td colSpan={span} className="px-3 py-6 text-center text-fg-muted">
        {children}
      </td>
    </tr>
  );
}
