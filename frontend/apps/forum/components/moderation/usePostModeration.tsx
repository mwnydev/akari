"use client";

import type { Schemas } from "@akari/api-client";
import type { LazyMenuItem } from "@akari/ui/lazy";
import { Ban, RotateCcw } from "lucide-react";
import dynamic from "next/dynamic";
import { useState, type ReactNode } from "react";
import { can, sanctionChoices } from "../../lib/staff";
import { useAfterSubmit } from "../composer/useAfterSubmit";

const RestoreDialog = dynamic(() => import("./RestoreDialog"), { ssr: false });
const SanctionDialog = dynamic(() => import("./SanctionDialog"), { ssr: false });

/** 楼层管理需要的上下文（只给管理人员传）。 */
export interface PostModeration {
  forumId: number;
  author: { id: number; name: string };
  staff: Schemas["StaffView"];
  isOwn: boolean;
}

/**
 * 楼层「…」菜单里的管理项：恢复被删除的楼层、处罚作者（禁言、禁拍、封号、封设备 / IP，按角色限时长）。
 * 按 /me 的权限只给本板块能做的；删除楼层沿用菜单里的「删除」（管理人员删除别人的楼层必须填理由）。
 * 返回菜单项与对应的弹窗（放在菜单旁边渲染）。
 */
export function usePostModeration(
  post: { id: number; floor: number; deleted: boolean },
  mod: PostModeration | undefined,
): { items: LazyMenuItem[]; dialogs: ReactNode } {
  const [dialog, setDialog] = useState<"restore" | "sanction" | null>(null);
  const afterSubmit = useAfterSubmit();
  if (!mod) return { items: [], dialogs: null };
  const close = () => setDialog(null);
  const items: LazyMenuItem[] = [];
  if (post.deleted && can(mod.staff, "post_delete", mod.forumId)) {
    items.push({
      label: "恢复楼层",
      icon: <RotateCcw size={16} aria-hidden />,
      separatorBefore: true,
      onSelect: () => setDialog("restore"),
    });
  }
  if (!mod.isOwn && sanctionChoices(mod.staff, mod.forumId).length > 0) {
    items.push({
      label: `处罚 ${mod.author.name}`,
      icon: <Ban size={16} aria-hidden />,
      danger: true,
      separatorBefore: items.length === 0,
      onSelect: () => setDialog("sanction"),
    });
  }
  const dialogs =
    dialog === "restore" ? (
      <RestoreDialog
        postId={post.id}
        floor={post.floor}
        onClose={close}
        onDone={() => {
          close();
          afterSubmit(null);
        }}
      />
    ) : dialog === "sanction" ? (
      <SanctionDialog user={mod.author} forumId={mod.forumId} staff={mod.staff} onClose={close} />
    ) : null;
  return { items, dialogs };
}
