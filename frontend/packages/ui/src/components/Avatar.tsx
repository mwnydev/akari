"use client";

import * as AvatarPrimitive from "@radix-ui/react-avatar";
import type { ReactNode } from "react";
import { cn } from "../cn";
import { avatarInitial } from "../initial";

export interface AvatarProps {
  name: string;
  src?: string | undefined;
  size?: number;
  /** undefined = 不显示在线点 */
  online?: boolean | undefined;
  /** 叠在右下角的内容（如浏览器里再取的在线点，见 OnlineDot） */
  badge?: ReactNode;
  className?: string;
}

/** 头像右下角的在线点；`avatarSize` 是所在头像的边长。 */
export function OnlineDot({ online, avatarSize }: { online: boolean; avatarSize: number }) {
  const dot = Math.max(8, Math.round(avatarSize / 4));
  return (
    <span
      role="img"
      aria-label={online ? "在线" : "离线"}
      className={cn(
        "absolute right-0 bottom-0 rounded-full ring-2 ring-surface",
        online ? "bg-online" : "bg-fg-muted/50",
      )}
      style={{ width: dot, height: dot }}
    />
  );
}

export function Avatar({ name, src, size = 40, online, badge, className }: AvatarProps) {
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
          {avatarInitial(name)}
        </AvatarPrimitive.Fallback>
      </AvatarPrimitive.Root>
      {online !== undefined ? <OnlineDot online={online} avatarSize={size} /> : null}
      {badge}
    </span>
  );
}

/** 一排小头像（主题列表的参与者），不叠放；超出 `max` 的人数显示为「+N」。 */
export function AvatarStack({
  people,
  max = 5,
  size = 24,
  className,
}: {
  people: { name: string; src?: string | undefined }[];
  max?: number;
  size?: number;
  className?: string;
}) {
  const shown = people.slice(0, max);
  const rest = people.length - shown.length;
  return (
    <span
      role="img"
      aria-label={`参与者：${people.map((p) => p.name).join("、")}`}
      className={cn("inline-flex items-center gap-0.5", className)}
    >
      {shown.map((p, i) => (
        <Avatar key={`${p.name}-${i}`} name={p.name} src={p.src} size={size} />
      ))}
      {rest > 0 ? (
        <span className="amount ml-0.5 text-xs text-fg-muted" aria-hidden>
          +{rest}
        </span>
      ) : null}
    </span>
  );
}
