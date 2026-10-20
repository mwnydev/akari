"use client";

import { EmptyState, LoadingState } from "@akari/ui";
import { SegmentedControl, Tabs, TabsContent, TabsList, TabsTrigger } from "@akari/ui/tabs";
import { useRouter } from "next/navigation";
import { useState, type ReactNode } from "react";
import { ListToolbar } from "./ListToolbar";
import { TopicListFrame, type TopicKind } from "./TopicTable";

export type FeedFilter = "all" | Exclude<TopicKind, "normal">;

export interface FeedRow {
  id: number;
  kind: TopicKind;
  pinned: boolean;
  /** 服务端渲染好的 TopicRow */
  node: ReactNode;
}

export type FeedList =
  | {
      value: string;
      label: string;
      rows: FeedRow[];
      /** 列表下方（分页、「更早的主题」） */
      footer?: ReactNode;
    }
  /** 按需加载的列表（热门）：页面没有取它，切到这个标签页时打开 `href`，由服务端渲染出这个列表 */
  | { value: string; label: string; href: string };

const FILTER_LABEL: Record<FeedFilter, string> = { all: "全部", trade: "交易", auction: "拍卖" };

/**
 * 主题列表的标签页：若干排序（最新 / 热门），可选的「全部 / 交易 / 拍卖」筛选在各排序之间共用，
 * 置顶帖不受筛选影响。`extra` 是额外的非主题标签页（首页的「板块」）。
 */
export function TopicFeed({
  lists,
  extra,
  defaultTab,
  filters = [],
  action,
  empty,
}: {
  lists: FeedList[];
  extra?: { value: string; label: string; node: ReactNode };
  defaultTab?: string | undefined;
  /** 只传有数据的类型；少于一个时不显示筛选 */
  filters?: Exclude<FeedFilter, "all">[];
  action?: ReactNode;
  /** 一条主题都没有时的提示 */
  empty: ReactNode;
}) {
  const router = useRouter();
  const [filter, setFilter] = useState<FeedFilter>("all");
  const pick = (rows: FeedRow[]) =>
    filter === "all" ? rows : rows.filter((r) => r.pinned || r.kind === filter);
  const options = (["all", ...filters] as FeedFilter[]).map((value) => ({
    value,
    label: FILTER_LABEL[value],
  }));
  const tabs = extra ? [...lists, extra] : lists;
  const [tab, setTab] = useState(defaultTab);
  // 选中的标签页不在了（例如切过去加载热门失败、页面不再给出这个列表）时回到第一个
  const current = tabs.some((t) => t.value === tab) ? tab : lists[0]?.value;
  const select = (value: string) => {
    setTab(value);
    const list = lists.find((l) => l.value === value);
    if (list && "href" in list) router.push(list.href, { scroll: false });
  };
  return (
    <Tabs value={current} onValueChange={select}>
      <ListToolbar action={action}>
        <TabsList variant="underline" aria-label="主题排序">
          {tabs.map((t) => (
            <TabsTrigger key={t.value} variant="underline" value={t.value}>
              {t.label}
            </TabsTrigger>
          ))}
        </TabsList>
        {filters.length ? (
          <SegmentedControl
            label="主题类型"
            value={filter}
            onValueChange={setFilter}
            options={options}
            className="mb-1.5"
          />
        ) : null}
      </ListToolbar>
      {lists.map((l) => {
        if ("href" in l) {
          return (
            <TabsContent key={l.value} value={l.value}>
              <LoadingState rows={3} label={`正在加载${l.label}主题`} />
            </TabsContent>
          );
        }
        const shown = pick(l.rows);
        return (
          <TabsContent key={l.value} value={l.value}>
            {l.rows.length === 0 ? (
              empty
            ) : shown.length ? (
              <TopicListFrame label={`${l.label}主题`}>{shown.map((r) => r.node)}</TopicListFrame>
            ) : (
              <EmptyState title="没有符合条件的主题" />
            )}
            {l.footer}
          </TabsContent>
        );
      })}
      {extra ? <TabsContent value={extra.value}>{extra.node}</TabsContent> : null}
    </Tabs>
  );
}
