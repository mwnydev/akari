"use client";

import * as PopoverPrimitive from "@radix-ui/react-popover";
import type { ComponentProps } from "react";
import { cn } from "../cn";

export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverClose = PopoverPrimitive.Close;

/** 浮动玻璃卡片（用户名片等）：贴着触发元素，点外面或按 Esc 关闭，焦点进出由 Radix 处理。 */
export function PopoverContent({
  className,
  sideOffset = 8,
  ...props
}: ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        sideOffset={sideOffset}
        collisionPadding={12}
        className={cn(
          "glass-strong z-50 w-[min(20rem,calc(100vw-24px))] animate-fade-in rounded-lg p-4 shadow-sheet",
          "focus-visible:outline-2 focus-visible:outline-accent",
          className,
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}
