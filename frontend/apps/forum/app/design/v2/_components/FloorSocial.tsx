"use client";

import { Button, Input } from "@akari/ui";
import { LazyMenu, type LazyMenuItem } from "@akari/ui/lazy";
import {
  Copy,
  EyeOff,
  Flag,
  Heart,
  Link2,
  MessageCircle,
  MoreHorizontal,
  Quote,
  Reply,
  Trash2,
} from "lucide-react";
import { lazy, Suspense, useRef, useState, type FormEvent } from "react";
import { useComposer } from "./ComposerContext";

const ReasonDialog = lazy(() => import("./ReasonDialog"));

export interface Person {
  id: number;
  name: string;
}

export interface CommentView {
  id: number;
  author: Person;
  replyTo?: Person;
  text: string;
}

/** 折叠前显示的评论条数。 */
const VISIBLE_COMMENTS = 5;
/** 楼中评论的点赞名单最多列出的人数，其余显示「等 N 人」。 */
const VISIBLE_LIKERS = 5;
/** 触屏长按多久弹出评论菜单（毫秒）。 */
const LONG_PRESS_MS = 500;
const REPORTED = "已举报，管理人员会尽快处理";

type Target = { kind: "floor" } | { kind: "comment"; to: Person };

function NameLink({ person }: { person: Person }) {
  return (
    <a href="#" className="font-medium text-accent hover:underline">
      {person.name}
    </a>
  );
}

/**
 * 楼层下方的社交区（朋友圈式）：右对齐的赞 / 评论 / 引用回复操作行，灰底评论块里先是点赞名单，再是平铺评论。
 * 只有两级：楼层 → 平铺评论；回复某条评论只是在前面加「回复 某人」。点评论即回复评论者，
 * 长按（触屏）、右键或「…」按钮打开评论菜单。
 */
export function FloorSocial({
  floor,
  floorAuthor,
  likers: initialLikers,
  likeCount: initialCount,
  comments: initialComments,
  viewer,
  canModerate,
}: {
  floor: number;
  floorAuthor: string;
  likers: Person[];
  likeCount: number;
  comments: CommentView[];
  viewer: Person;
  canModerate: boolean;
}) {
  const { openComposer } = useComposer();
  // 访问者可能已在点赞名单里：以此为初始状态，名单里只出现一次
  const likedBefore = initialLikers.some((p) => p.id === viewer.id);
  const [liked, setLiked] = useState(likedBefore);
  const [comments, setComments] = useState(initialComments);
  const [expanded, setExpanded] = useState(false);
  const [target, setTarget] = useState<Target | null>(null);
  const [draft, setDraft] = useState("");
  const [notice, setNotice] = useState("");
  const opener = useRef<HTMLElement | null>(null);
  const nextId = useRef(initialComments.length + 1);

  const others = initialLikers.filter((p) => p.id !== viewer.id);
  const likers = liked ? [viewer, ...others] : others;
  const likeCount = initialCount - Number(likedBefore) + Number(liked);
  const shownLikers = likers.slice(0, VISIBLE_LIKERS);
  const shownComments = expanded ? comments : comments.slice(0, VISIBLE_COMMENTS);
  const hiddenCount = comments.length - shownComments.length;

  const startReply = (t: Target) => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setTarget(t);
    setDraft("");
  };
  const closeReply = () => {
    setTarget(null);
    opener.current?.focus();
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text || !target) return;
    const id = nextId.current++;
    setComments((cs) => [
      ...cs,
      { id, author: viewer, ...(target.kind === "comment" ? { replyTo: target.to } : {}), text },
    ]);
    setExpanded(true);
    setTarget(null);
    setDraft("");
  };

  const placeholder = target?.kind === "comment" ? `回复 ${target.to.name}：` : "评论…";

  return (
    <div className="mt-3">
      <div className="-mr-2 flex flex-wrap items-center justify-end gap-0.5">
        <Button variant="subtle" size="sm" aria-pressed={liked} onClick={() => setLiked((v) => !v)}>
          <Heart size={16} aria-hidden fill={liked ? "currentColor" : "none"} />
          {likeCount > 0 ? <span className="amount">{likeCount}</span> : null}
          {liked ? "已赞" : "赞"}
        </Button>
        <Button variant="subtle" size="sm" onClick={() => startReply({ kind: "floor" })}>
          <MessageCircle size={16} aria-hidden />
          评论
        </Button>
        <Button
          variant="subtle"
          size="sm"
          onClick={() => openComposer({ floor, author: floorAuthor })}
        >
          <Quote size={16} aria-hidden />
          引用回复
        </Button>
        <FloorMenu floor={floor} canModerate={canModerate} onNotice={setNotice} />
      </div>

      {likeCount > 0 || comments.length > 0 ? (
        <div className="mt-2 rounded-md bg-wash text-sm leading-relaxed">
          {likeCount > 0 ? (
            <p className="flex gap-1.5 px-3 py-1.5">
              <Heart
                size={14}
                aria-hidden
                fill="currentColor"
                className="mt-1 shrink-0 text-accent"
              />
              <span>
                <span className="sr-only">赞过的人：</span>
                {shownLikers.map((p, i) => (
                  <span key={p.id}>
                    {i > 0 ? "、" : null}
                    <NameLink person={p} />
                  </span>
                ))}
                {likeCount > shownLikers.length ? (
                  <span className="text-fg-muted"> 等 {likeCount} 人</span>
                ) : null}
              </span>
            </p>
          ) : null}
          {comments.length > 0 ? (
            <ul
              aria-label={`${comments.length} 条评论`}
              className={likeCount > 0 ? "border-t border-border px-3 py-1.5" : "px-3 py-1.5"}
            >
              {shownComments.map((c) => (
                <CommentItem
                  key={c.id}
                  comment={c}
                  canDelete={canModerate || c.author.id === viewer.id}
                  asModerator={c.author.id !== viewer.id}
                  onReply={() => startReply({ kind: "comment", to: c.author })}
                  onDelete={() => setComments((cs) => cs.filter((x) => x.id !== c.id))}
                  onNotice={setNotice}
                />
              ))}
              {hiddenCount > 0 ? (
                <li>
                  <button
                    type="button"
                    className="rounded-sm py-0.5 font-medium text-accent hover:underline focus-visible:outline-2 focus-visible:outline-accent"
                    onClick={() => setExpanded(true)}
                  >
                    展开其余 {hiddenCount} 条
                  </button>
                </li>
              ) : null}
            </ul>
          ) : null}
        </div>
      ) : null}

      {target ? (
        <form className="mt-2 flex items-center gap-2" onSubmit={submit}>
          <Input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") closeReply();
            }}
            aria-label={target.kind === "comment" ? `回复 ${target.to.name}` : `评论 #${floor}`}
            placeholder={placeholder}
            maxLength={500}
            className="h-9"
          />
          <Button type="submit" size="sm" disabled={!draft.trim()}>
            发送
          </Button>
          <Button variant="subtle" size="sm" onClick={closeReply}>
            取消
          </Button>
        </form>
      ) : null}
      <p role="status" className="text-xs text-fg-muted empty:hidden">
        {notice}
      </p>
    </div>
  );
}

function CommentItem({
  comment: c,
  canDelete,
  asModerator,
  onReply,
  onDelete,
  onNotice,
}: {
  comment: CommentView;
  canDelete: boolean;
  /** 删除别人的评论属于管理操作，要填理由 */
  asModerator: boolean;
  onReply: () => void;
  onDelete: () => void;
  onNotice: (s: string) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  const longPressed = useRef(false);
  const cancel = () => window.clearTimeout(timer.current);

  const items: LazyMenuItem[] = [
    { label: `回复 ${c.author.name}`, icon: <Reply size={16} aria-hidden />, onSelect: onReply },
    {
      label: "复制",
      icon: <Copy size={16} aria-hidden />,
      onSelect: () => {
        void navigator.clipboard?.writeText(c.text);
        onNotice("已复制评论内容");
      },
    },
    { label: "举报", icon: <Flag size={16} aria-hidden />, onSelect: () => onNotice(REPORTED) },
  ];
  if (canDelete) {
    items.push({
      label: asModerator ? "删除（管理操作）" : "删除",
      icon: <Trash2 size={16} aria-hidden />,
      danger: true,
      separatorBefore: true,
      onSelect: asModerator ? () => setConfirmDelete(true) : onDelete,
    });
  }

  return (
    <>
      <li
        className="group -mx-1.5 flex cursor-pointer items-start gap-1 rounded-sm px-1.5 [-webkit-touch-callout:none] hover:bg-fg/5"
        onClick={(e) => {
          if (longPressed.current) {
            longPressed.current = false;
            return;
          }
          // 菜单弹层经 React 冒泡上来但不在这个 DOM 里；点名字是去主页，点「…」是菜单；其余位置都是回复
          const el = e.target as HTMLElement;
          if (!e.currentTarget.contains(el) || el.closest("a, button")) return;
          onReply();
        }}
        onPointerDown={(e) => {
          if (e.pointerType === "mouse") return;
          longPressed.current = false;
          timer.current = window.setTimeout(() => {
            longPressed.current = true;
            setMenuOpen(true);
          }, LONG_PRESS_MS);
        }}
        onPointerUp={cancel}
        onPointerLeave={cancel}
        onPointerCancel={cancel}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenuOpen(true);
        }}
      >
        <p className="min-w-0 flex-1 py-0.5">
          <NameLink person={c.author} />
          {c.replyTo ? (
            <>
              <span className="text-fg-muted"> 回复 </span>
              <NameLink person={c.replyTo} />
            </>
          ) : null}
          <span className="text-fg-muted">：</span>
          {c.text}
        </p>
        <LazyMenu
          align="end"
          open={menuOpen}
          onOpenChange={setMenuOpen}
          items={items}
          trigger={
            <button
              type="button"
              aria-label={`${c.author.name} 的评论：更多操作`}
              className="mt-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded-sm text-fg-muted opacity-0 group-hover:opacity-100 hover:bg-surface-2 focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent data-[state=open]:opacity-100"
            >
              <MoreHorizontal size={16} aria-hidden />
            </button>
          }
        />
      </li>
      {confirmDelete ? (
        <Suspense fallback={null}>
          <ReasonDialog
            open
            onOpenChange={setConfirmDelete}
            title={`删除 ${c.author.name} 的评论`}
            confirmLabel="删除"
            danger
            onConfirm={onDelete}
          />
        </Suspense>
      ) : null}
    </>
  );
}

function FloorMenu({
  floor,
  canModerate,
  onNotice,
}: {
  floor: number;
  canModerate: boolean;
  onNotice: (s: string) => void;
}) {
  const [modAction, setModAction] = useState<"hide" | "delete" | null>(null);
  const items: LazyMenuItem[] = [
    {
      label: "复制楼层链接",
      icon: <Link2 size={16} aria-hidden />,
      onSelect: () => {
        void navigator.clipboard?.writeText(
          `${location.origin}${location.pathname}#floor-${floor}`,
        );
        onNotice("已复制楼层链接");
      },
    },
    { label: "举报", icon: <Flag size={16} aria-hidden />, onSelect: () => onNotice(REPORTED) },
  ];
  if (canModerate) {
    items.push(
      {
        label: "隐藏楼层",
        icon: <EyeOff size={16} aria-hidden />,
        separatorBefore: true,
        onSelect: () => setModAction("hide"),
      },
      {
        label: "删除楼层",
        icon: <Trash2 size={16} aria-hidden />,
        danger: true,
        onSelect: () => setModAction("delete"),
      },
    );
  }
  return (
    <>
      <LazyMenu
        items={items}
        trigger={
          <Button variant="subtle" size="sm" aria-label={`#${floor} 更多操作`} className="px-2">
            <MoreHorizontal size={16} aria-hidden />
          </Button>
        }
      />
      {modAction ? (
        <Suspense fallback={null}>
          <ReasonDialog
            open
            onOpenChange={(o) => {
              if (!o) setModAction(null);
            }}
            title={modAction === "hide" ? `隐藏 #${floor}` : `删除 #${floor}`}
            confirmLabel={modAction === "hide" ? "隐藏" : "删除"}
            danger={modAction === "delete"}
          />
        </Suspense>
      ) : null}
    </>
  );
}
