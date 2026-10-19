"use client";

import * as AvatarPrimitive from "@radix-ui/react-avatar";
import { cn } from "../cn";

export interface AvatarProps {
  name: string;
  src?: string | undefined;
  size?: number;
  /** undefined = 不显示在线点 */
  online?: boolean | undefined;
  className?: string;
}

export function Avatar({ name, src, size = 40, online, className }: AvatarProps) {
  const dot = Math.max(8, Math.round(size / 4));
  return (
    <span
      className={cn("relative inline-flex shrink-0", className)}
      style={{ width: size, height: size }}
    >
      <AvatarPrimitive.Root className="inline-flex size-full overflow-hidden rounded-full bg-surface-2">
        {src ? (
          <AvatarPrimitive.Image src={src} alt={name} className="size-full object-cover" />
        ) : null}
        <AvatarPrimitive.Fallback
          className="flex size-full items-center justify-center font-medium text-fg-muted"
          style={{ fontSize: size * 0.42 }}
        >
          {Array.from(name)[0] ?? "?"}
        </AvatarPrimitive.Fallback>
      </AvatarPrimitive.Root>
      {online !== undefined ? (
        <span
          role="img"
          aria-label={online ? "在线" : "离线"}
          className={cn(
            "absolute right-0 bottom-0 rounded-full ring-2 ring-surface",
            online ? "bg-online" : "bg-fg-muted/50",
          )}
          style={{ width: dot, height: dot }}
        />
      ) : null}
    </span>
  );
}
