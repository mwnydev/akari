"use client";

import { EmptyState } from "@akari/ui";
import { SegmentedControl, Tabs, TabsContent, TabsList, TabsTrigger } from "@akari/ui/tabs";
import { useState, type ReactNode } from "react";
import { ListToolbar } from "./ListToolbar";
import { TopicListFrame } from "./TopicList";

type Filter = "all" | "trade" | "auction";

export interface FeedRow {
  id: number;
  kind: "normal" | "trade" | "auction";
  pinned: boolean;
  /** 服务端渲染好的 TopicRow */
  node: ReactNode;
}

const FILTERS = [
  { value: "all", label: "全部" },
  { value: "trade", label: "交易" },
  { value: "auction", label: "拍卖" },
] as const satisfies readonly { value: Filter; label: string }[];

/** 板块页列表：最新 / 热门两个排序，交易 / 拍卖筛选在两者之间共用。置顶帖不受筛选影响。 */
export function BoardFeed({ latest, hot }: { latest: FeedRow[]; hot: FeedRow[] }) {
  const [filter, setFilter] = useState<Filter>("all");
  const pick = (rows: FeedRow[]) =>
    filter === "all" ? rows : rows.filter((r) => r.pinned || r.kind === filter);
  const list = (rows: FeedRow[], label: string) => {
    const shown = pick(rows);
    return shown.length > 0 ? (
      <TopicListFrame label={label}>{shown.map((r) => r.node)}</TopicListFrame>
    ) : (
      <EmptyState title="没有符合条件的主题" />
    );
  };
  return (
    <Tabs defaultValue="latest">
      <ListToolbar newLabel="发交易帖">
        <TabsList variant="underline" aria-label="主题排序">
          <TabsTrigger variant="underline" value="latest">
            最新
          </TabsTrigger>
          <TabsTrigger variant="underline" value="hot">
            热门
          </TabsTrigger>
        </TabsList>
        <SegmentedControl
          label="主题类型"
          value={filter}
          onValueChange={setFilter}
          options={[...FILTERS]}
          className="mb-1.5"
        />
      </ListToolbar>
      <TabsContent value="latest">{list(latest, "最新主题")}</TabsContent>
      <TabsContent value="hot">{list(hot, "热门主题")}</TabsContent>
    </Tabs>
  );
}
