"use client";

import { Button } from "@akari/ui";
import { History, Pencil, Trash2 } from "lucide-react";
import dynamic from "next/dynamic";
import { useState, type ReactNode } from "react";
import type { UserCardView } from "../../lib/display";
import { useAfterSubmit } from "../composer/useAfterSubmit";

const Composer = dynamic(() => import("../composer/Composer"), { ssr: false });
const DeleteDialog = dynamic(() => import("./DeleteDialog"), { ssr: false });
const HistoryDialog = dynamic(() => import("./HistoryDialog"), { ssr: false });

export interface FloorPermissions {
  /** 作者本人（主题未锁定）或管理人员 */
  canEdit: boolean;
  /** 管理人员处理别人的楼层：删除必须填理由 */
  asModerator: boolean;
  /** 已编辑过且访问者能看历史 */
  hasHistory: boolean;
}

/** 楼层里的交互部分：编辑（原地换成编辑器）、删除、编辑历史。正文由服务端渲染后作为 children 传入。 */
export function FloorShell({
  postId,
  floor,
  title,
  perms,
  mentionable,
  meta,
  children,
}: {
  postId: number;
  floor: number;
  /** 1 楼编辑时可以一起改标题 */
  title?: string | undefined;
  perms: FloorPermissions;
  mentionable: UserCardView[];
  meta: ReactNode;
  children: ReactNode;
}) {
  const [editing, setEditing] = useState(false);
  const [dialog, setDialog] = useState<"delete" | "history" | null>(null);
  const afterSubmit = useAfterSubmit();
  const close = () => setDialog(null);

  return (
    <>
      <div className="mt-3">
        {editing ? (
          <Composer
            target={{ kind: "edit", postId, floor, title }}
            mentionable={mentionable}
            onCancel={() => setEditing(false)}
            onDone={(href) => {
              setEditing(false);
              afterSubmit(href);
            }}
          />
        ) : (
          children
        )}
      </div>
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-fg-muted">
        <span>{meta}</span>
        <span className="flex gap-1">
          {perms.hasHistory ? (
            <Button variant="ghost" size="sm" onClick={() => setDialog("history")}>
              <History size={14} aria-hidden />
              编辑历史
            </Button>
          ) : null}
          {perms.canEdit && !editing ? (
            <>
              <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
                <Pencil size={14} aria-hidden />
                编辑
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setDialog("delete")}>
                <Trash2 size={14} aria-hidden />
                删除
              </Button>
            </>
          ) : null}
        </span>
      </div>
      {dialog === "delete" ? (
        <DeleteDialog
          postId={postId}
          floor={floor}
          needsReason={perms.asModerator}
          onClose={close}
          onDeleted={() => {
            close();
            afterSubmit(null);
          }}
        />
      ) : null}
      {dialog === "history" ? (
        <HistoryDialog postId={postId} floor={floor} onClose={close} />
      ) : null}
    </>
  );
}
