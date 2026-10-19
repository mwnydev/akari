"use client";

import { errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { LazyMenu, type LazyMenuItem } from "@akari/ui/lazy";
import {
  Flag,
  History,
  Link2,
  MessageCircle,
  MoreHorizontal,
  Pencil,
  Quote,
  Trash2,
} from "lucide-react";
import dynamic from "next/dynamic";
import { useState, type ReactNode } from "react";
import type { UserCardView } from "../../lib/display";
import type { PostSocial, SocialPerson } from "../../lib/floor-social";
import { reportPost } from "../../lib/floor-social-client";
import { useAfterSubmit } from "../composer/useAfterSubmit";
import { usePostModeration, type PostModeration } from "../moderation/usePostModeration";
import { useComposer } from "./ComposerContext";
import { FloorSocialBlock, LikeButton, useFloorSocial } from "./FloorSocial";
import { usePreview } from "./preview";

const Composer = dynamic(() => import("../composer/Composer"), { ssr: false });
const DeleteDialog = dynamic(() => import("../topic/DeleteDialog"), { ssr: false });
const HistoryDialog = dynamic(() => import("../topic/HistoryDialog"), { ssr: false });

/** 引用摘录最长字数。 */
const QUOTE_CHARS = 140;

export interface FloorPermissions {
  /** 作者本人（主题未锁定）或管理人员 */
  canEdit: boolean;
  /** 管理人员处理别人的楼层：删除必须填理由 */
  asModerator: boolean;
  /** 已编辑过且访问者能看历史 */
  hasHistory: boolean;
  /** 访问者自己的楼层（不给「举报」） */
  own?: boolean;
}

/** 引用的摘录：访问者在这层楼里选中了文字就用选中的，否则取正文开头。 */
function quoteExcerpt(floor: number): string {
  const body = document.querySelector(`#floor-${floor} [data-floor-body]`);
  const sel = window.getSelection();
  const picked =
    sel && !sel.isCollapsed && body?.contains(sel.anchorNode) ? sel.toString() : body?.textContent;
  const text = (picked ?? "").replace(/\s+/g, " ").trim();
  return text.length > QUOTE_CHARS ? `${text.slice(0, QUOTE_CHARS)}…` : text;
}

/**
 * 楼层里的交互部分：正文（编辑时原地换成编辑器）、嵌入内容、右对齐的操作行（赞、评论、引用回复、编辑、「…」），
 * 以及下方的点赞评论块。正文由服务端渲染后作为 children 传入。
 * `social` 为空（已删除的楼层）时不显示赞和评论；`viewer` 为空（游客）时只读。
 * `hidden`：被管理人员隐藏的楼层（与已删除的一样不能点赞、评论、引用、编辑、举报，管理人员仍可删除）。
 * `canModerate` = 能删除别人的评论；`mod` 给管理人员，「…」菜单里多出隐藏、恢复楼层、处罚作者。
 */
export function FloorContent({
  topicId,
  postId,
  floor,
  author,
  quoteHref,
  title,
  deleted = false,
  hidden = false,
  perms,
  mentionable = [],
  social,
  viewer,
  canModerate = false,
  canReply = false,
  mod,
  extra,
  children,
}: {
  topicId: number;
  postId: number;
  floor: number;
  author: string;
  /** 「引用回复」引用块里链接回这层楼的地址 */
  quoteHref: string;
  /** 1 楼编辑时可以一起改标题 */
  title?: string | undefined;
  deleted?: boolean;
  hidden?: boolean;
  perms?: FloorPermissions | undefined;
  mentionable?: UserCardView[];
  social: PostSocial | null;
  viewer: SocialPerson | null;
  canModerate?: boolean;
  canReply?: boolean;
  mod?: PostModeration | undefined;
  extra?: ReactNode;
  children: ReactNode;
}) {
  const [editing, setEditing] = useState(false);
  const [dialog, setDialog] = useState<"delete" | "history" | null>(null);
  const [notice, setNotice] = useState("");
  const afterSubmit = useAfterSubmit();
  const composer = useComposer();
  const close = () => setDialog(null);
  const frozen = deleted || hidden;
  const canEdit = (perms?.canEdit ?? false) && !frozen;
  // 隐藏的楼层只有管理人员能删除
  const canDelete =
    (perms?.canEdit ?? false) && !deleted && (!hidden || (perms?.asModerator ?? false));
  const moderation = usePostModeration({ id: postId, floor, deleted, hidden }, mod);
  const own = perms?.own ?? false;
  const preview = usePreview();

  const menu: LazyMenuItem[] = [
    {
      label: "复制楼层链接",
      icon: <Link2 size={16} aria-hidden />,
      onSelect: () => {
        void navigator.clipboard?.writeText(new URL(quoteHref, location.href).toString());
        setNotice("已复制楼层链接");
      },
    },
  ];
  if (perms?.hasHistory) {
    menu.push({
      label: "编辑历史",
      icon: <History size={16} aria-hidden />,
      onSelect: () => setDialog("history"),
    });
  }
  if (viewer && !own && !frozen) {
    menu.push({
      label: "举报",
      icon: <Flag size={16} aria-hidden />,
      onSelect: () => {
        (preview ? Promise.resolve() : reportPost(postId)).then(
          () => setNotice("已举报，管理人员会尽快处理"),
          (e: unknown) => setNotice(errorMessage(e)),
        );
      },
    });
  }
  if (canDelete) {
    menu.push({
      label: "删除",
      icon: <Trash2 size={16} aria-hidden />,
      danger: true,
      separatorBefore: true,
      onSelect: () => setDialog("delete"),
    });
  }
  menu.push(...moderation.items);

  const actions = (
    <>
      {canReply && composer && !frozen ? (
        <Button
          variant="subtle"
          size="sm"
          onClick={() =>
            composer.openComposer({ floor, author, href: quoteHref, text: quoteExcerpt(floor) })
          }
        >
          <Quote size={16} aria-hidden />
          引用回复
        </Button>
      ) : null}
      {canEdit && !editing ? (
        <Button variant="subtle" size="sm" onClick={() => setEditing(true)}>
          <Pencil size={16} aria-hidden />
          编辑
        </Button>
      ) : null}
      <LazyMenu
        align="end"
        items={menu}
        trigger={
          <Button variant="subtle" size="sm" aria-label={`#${floor} 更多操作`} className="px-2">
            <MoreHorizontal size={16} aria-hidden />
          </Button>
        }
      />
    </>
  );

  return (
    <>
      <div className="mt-2" data-floor-body>
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
      {extra && !frozen ? <div className="mt-3">{extra}</div> : null}
      {social && !frozen ? (
        <SocialRow
          topicId={topicId}
          postId={postId}
          floor={floor}
          social={social}
          viewer={viewer}
          canModerate={canModerate}
        >
          {actions}
        </SocialRow>
      ) : (
        <div className="mt-3 -mr-2 flex flex-wrap items-center justify-end gap-0.5">{actions}</div>
      )}
      <p role="status" className="text-right text-xs text-fg-muted empty:hidden">
        {notice}
      </p>
      {dialog === "delete" ? (
        <DeleteDialog
          postId={postId}
          floor={floor}
          needsReason={perms?.asModerator ?? false}
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
      {moderation.dialogs}
    </>
  );
}

/** 有点赞评论数据时的操作行：赞、评论在前，其余按钮（children）在后，下面是评论块。 */
function SocialRow({
  topicId,
  postId,
  floor,
  social: initial,
  viewer,
  canModerate,
  children,
}: {
  topicId: number;
  postId: number;
  floor: number;
  social: PostSocial;
  viewer: SocialPerson | null;
  canModerate: boolean;
  children: ReactNode;
}) {
  const social = useFloorSocial({ topicId, postId, initial, viewer });
  return (
    <div className="mt-3">
      <div className="-mr-2 flex flex-wrap items-center justify-end gap-0.5">
        {viewer ? (
          <>
            <LikeButton social={social} />
            <Button variant="subtle" size="sm" onClick={() => social.startReply({ kind: "floor" })}>
              <MessageCircle size={16} aria-hidden />
              评论
            </Button>
          </>
        ) : null}
        {children}
      </div>
      <FloorSocialBlock
        social={social}
        postId={postId}
        floor={floor}
        viewer={viewer}
        canModerate={canModerate}
      />
    </div>
  );
}
