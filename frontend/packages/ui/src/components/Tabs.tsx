"use client";

import * as TabsPrimitive from "@radix-ui/react-tabs";
import * as ToggleGroup from "@radix-ui/react-toggle-group";
import type { ComponentProps } from "react";
import { cn } from "../cn";

export const Tabs = TabsPrimitive.Root;
export const TabsContent = TabsPrimitive.Content;

const segmentList =
  "inline-flex max-w-full items-center gap-0.5 overflow-x-auto rounded-pill bg-surface-2 p-1";
const segmentItem =
  "shrink-0 rounded-pill px-3.5 py-1 text-sm text-fg-muted transition-colors duration-150 " +
  "hover:text-fg data-[state=active]:bg-surface data-[state=active]:font-medium data-[state=active]:text-fg " +
  "data-[state=on]:bg-surface data-[state=on]:font-medium data-[state=on]:text-fg data-[state=active]:shadow-float data-[state=on]:shadow-float " +
  "focus-visible:outline-2 focus-visible:outline-accent";

// 下划线样式（论坛列表顶部的「最新 / 热门」）：纯文字，选中项只换颜色和下划线，字重不变以免跳版。
// 整条列表下移 1px 压在外层的分隔线上（而不是每个标签各自 -mb-px：那样标签比列表高 1px，横向可滚的列表
// 在纵向也溢出，Windows 常驻滚动条会在标签旁边画出一条竖滚动条）；纵向不滚动，横向放不下时才出细滚动条。
const underlineList =
  "-mb-px flex max-w-full items-center gap-1 overflow-x-auto overflow-y-hidden [scrollbar-width:thin]";
const underlineItem =
  "shrink-0 border-b-2 border-transparent px-3 py-1.5 text-fg-muted transition-colors duration-150 " +
  "hover:text-fg data-[state=active]:border-accent-fill data-[state=active]:text-fg " +
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent";

export type TabsVariant = "segment" | "underline";

export function TabsList({
  variant = "segment",
  className,
  ...props
}: ComponentProps<typeof TabsPrimitive.List> & { variant?: TabsVariant }) {
  return (
    <TabsPrimitive.List
      className={cn(variant === "underline" ? underlineList : segmentList, className)}
      {...props}
    />
  );
}

export function TabsTrigger({
  variant = "segment",
  className,
  ...props
}: ComponentProps<typeof TabsPrimitive.Trigger> & { variant?: TabsVariant }) {
  return (
    <TabsPrimitive.Trigger
      className={cn(variant === "underline" ? underlineItem : segmentItem, className)}
      {...props}
    />
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
}

/** 分段控件：单选、不可取消选择。用于筛选，不切换页面内容区时比 Tabs 更合适。 */
export function SegmentedControl<T extends string>({
  options,
  value,
  onValueChange,
  label,
  className,
}: {
  options: SegmentOption<T>[];
  value: T;
  onValueChange: (v: T) => void;
  label: string;
  className?: string;
}) {
  return (
    <ToggleGroup.Root
      type="single"
      aria-label={label}
      value={value}
      onValueChange={(v) => {
        const hit = options.find((o) => o.value === v);
        if (hit) onValueChange(hit.value);
      }}
      className={cn(segmentList, className)}
    >
      {options.map((o) => (
        <ToggleGroup.Item key={o.value} value={o.value} className={segmentItem}>
          {o.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  );
}
