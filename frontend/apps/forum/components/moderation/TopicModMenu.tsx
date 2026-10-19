"use client";

import type { Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import { LazyMenu } from "@akari/ui/lazy";
import { EyeOff, FolderInput, Lock, Pin, Shield, Star, type LucideIcon } from "lucide-react";
import dynamic from "next/dynamic";
import { useState } from "react";
import { can } from "../../lib/staff";
import { useAfterSubmit } from "../composer/useAfterSubmit";
import { topicActions, type TopicAction, type TopicActionItem } from "./topic-actions";

const TopicModDialog = dynamic(() => import("./TopicModDialog"), { ssr: false });

const ICON: Record<TopicAction, LucideIcon> = {
  pin: Pin,
  unpin: Pin,
  feature: Star,
  unfeature: Star,
  lock: Lock,
  unlock: Lock,
  move: FolderInput,
  hide: EyeOff,
  unhide: EyeOff,
};

/**
 * 主题页的管理菜单（ADR 0013：没有单独的「管理模式」，版主与管理员就在论坛里操作）。
 * 按 /me 的权限只列出本板块能做的操作；每项都要选理由。
 */
export function TopicModMenu({
  topicId,
  forumId,
  flags,
  forums,
  staff,
}: {
  topicId: number;
  forumId: number;
  flags: Schemas["TopicFlags"];
  forums: Pick<Schemas["ForumView"], "id" | "name">[];
  staff: Schemas["StaffView"] | null | undefined;
}) {
  const [current, setCurrent] = useState<TopicActionItem | null>(null);
  const afterSubmit = useAfterSubmit();
  // 移动要求目标板块也有权限
  const targets = forums.filter((f) => f.id !== forumId && can(staff, "topic_moderate", f.id));
  const items = topicActions(staff, forumId, flags, targets.length > 0);
  if (!items.length) return null;
  return (
    <>
      <LazyMenu
        align="end"
        heading="管理操作 · 需填理由"
        trigger={
          <Button variant="secondary" size="sm">
            <Shield size={16} aria-hidden />
            管理
          </Button>
        }
        items={items.map((it) => {
          const Icon = ICON[it.action];
          return {
            label: it.label,
            icon: <Icon size={16} aria-hidden />,
            danger: it.danger,
            separatorBefore: it.action === "hide" || it.action === "unhide",
            onSelect: () => setCurrent(it),
          };
        })}
      />
      {current ? (
        <TopicModDialog
          topicId={topicId}
          item={current}
          targets={targets}
          onClose={() => setCurrent(null)}
          onDone={() => {
            setCurrent(null);
            afterSubmit(null);
          }}
        />
      ) : null}
    </>
  );
}
