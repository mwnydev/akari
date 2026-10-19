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

export function TabsList({ className, ...props }: ComponentProps<typeof TabsPrimitive.List>) {
  return <TabsPrimitive.List className={cn(segmentList, className)} {...props} />;
}

export function TabsTrigger({ className, ...props }: ComponentProps<typeof TabsPrimitive.Trigger>) {
  return <TabsPrimitive.Trigger className={cn(segmentItem, className)} {...props} />;
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
