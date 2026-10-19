"use client";

import type { Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import { LazyMenu } from "@akari/ui/lazy";
import { Ban, MoreHorizontal, RotateCcw } from "lucide-react";
import dynamic from "next/dynamic";
import { useState } from "react";
import { can, sanctionChoices } from "../../lib/staff";
import { useAfterSubmit } from "../composer/useAfterSubmit";

const RestoreDialog = dynamic(() => import("./RestoreDialog"), { ssr: false });
const SanctionDialog = dynamic(() => import("./SanctionDialog"), { ssr: false });

/**
 * 楼层的管理菜单：恢复被删除的楼层、处罚作者（禁言、禁拍、封号、封设备 / IP，按角色限时长）。
 * 删除楼层沿用楼层下方的「删除」（管理人员删除别人的楼层必须填理由）。没有任何可做的操作时不渲染。
 */
export function PostModMenu({
  postId,
  floor,
  forumId,
  deleted,
  author,
  staff,
  isOwn,
}: {
  postId: number;
  floor: number;
  forumId: number;
  deleted: boolean;
  author: { id: number; name: string };
  staff: Schemas["StaffView"] | null | undefined;
  isOwn: boolean;
}) {
  const [dialog, setDialog] = useState<"restore" | "sanction" | null>(null);
  const afterSubmit = useAfterSubmit();
  const canRestore = deleted && can(staff, "post_delete", forumId);
  const canSanction = !isOwn && sanctionChoices(staff, forumId).length > 0;
  if (!staff || (!canRestore && !canSanction)) return null;
  const close = () => setDialog(null);
  return (
    <>
      <LazyMenu
        align="end"
        heading={`#${floor} 管理`}
        trigger={
          <Button variant="subtle" size="sm" className="px-2">
            <MoreHorizontal size={16} aria-hidden />
            <span className="sr-only">管理 #{floor}</span>
          </Button>
        }
        items={[
          ...(canRestore
            ? [
                {
                  label: "恢复楼层",
                  icon: <RotateCcw size={16} aria-hidden />,
                  onSelect: () => setDialog("restore"),
                },
              ]
            : []),
          ...(canSanction
            ? [
                {
                  label: `处罚 ${author.name}`,
                  icon: <Ban size={16} aria-hidden />,
                  danger: true,
                  onSelect: () => setDialog("sanction"),
                },
              ]
            : []),
        ]}
      />
      {dialog === "restore" ? (
        <RestoreDialog
          postId={postId}
          floor={floor}
          onClose={close}
          onDone={() => {
            close();
            afterSubmit(null);
          }}
        />
      ) : null}
      {dialog === "sanction" ? (
        <SanctionDialog user={author} forumId={forumId} staff={staff} onClose={close} />
      ) : null}
    </>
  );
}
