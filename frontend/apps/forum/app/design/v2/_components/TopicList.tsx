import type { ReactNode } from "react";
import type { TopicSummary } from "../_mock/data";
import { TopicRow } from "./TopicRow";

const th = "px-2 py-2 font-normal";

/** 主题表格：没有外框，表头是灰色小字，行之间 1px 分隔线。子元素是若干 TopicRow（筛选后的列表由调用方传入）。 */
export function TopicListFrame({ label, children }: { label: string; children: ReactNode }) {
  return (
    <table className="w-full border-collapse">
      <caption className="sr-only">{label}</caption>
      <thead className="hidden md:table-header-group">
        <tr className="border-b border-border text-left text-sm text-fg-muted">
          <th scope="col" className={th}>
            主题
          </th>
          <th scope="col" className={`${th} w-40`}>
            参与者
          </th>
          <th scope="col" className={`${th} w-16 text-right`}>
            回复
          </th>
          <th scope="col" className={`${th} w-16 text-right`}>
            浏览
          </th>
          <th scope="col" className={`${th} w-20 text-right`}>
            活动
          </th>
        </tr>
      </thead>
      <tbody>{children}</tbody>
    </table>
  );
}

export function TopicList({ topics, label }: { topics: TopicSummary[]; label: string }) {
  return (
    <TopicListFrame label={label}>
      {topics.map((t) => (
        <TopicRow key={t.id} topic={t} />
      ))}
    </TopicListFrame>
  );
}
