"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Input } from "@akari/ui";
import { LazyMenu, type LazyMenuItem } from "@akari/ui/lazy";
import { Copy, Flag, Heart, MoreHorizontal, Reply, RotateCcw, Trash2 } from "lucide-react";
import { lazy, Suspense, useEffect, useReducer, useRef, useState, type FormEvent } from "react";
import { profileHref, userName } from "../../lib/display";
import {
  commentView,
  socialPerson,
  type PostComment,
  type PostSocial,
  type SocialEvent,
  type SocialPerson,
} from "../../lib/floor-social";
import {
  createComment,
  deleteComment,
  fetchPeople,
  listComments,
  reportComment,
  restoreComment,
  setLike,
  watchTopicSocial,
  type TopicSocialEvent,
} from "../../lib/floor-social-client";
import { usePostCaptcha } from "../captcha/PostCaptcha";
import { usePreview } from "./preview";
import { initialSocial, MAX_LIKERS, socialReducer, type CommentEntry } from "./social-state";

const CommentModDialog = lazy(() => import("../moderation/CommentModDialog"));

/** 折叠前显示的评论条数（接口也只随楼层带回前 5 条）。 */
const VISIBLE_COMMENTS = 5;
/** 触屏长按多久弹出评论菜单（毫秒）。 */
const LONG_PRESS_MS = 500;
/** 评论最长字数。 */
const MAX_COMMENT = 500;
const REPORTED = "已举报，管理人员会尽快处理";

type Target = { kind: "floor" } | { kind: "comment"; comment: PostComment };

function NameLink({ person }: { person: SocialPerson }) {
  return (
    <a href={profileHref(person)} className="font-medium text-accent hover:underline">
      {userName(person)}
    </a>
  );
}

/** 一层楼里已经知道名字的人：点赞名单、评论作者与被回复的人、访问者。 */
function knownPeople(initial: PostSocial, viewer: SocialPerson | null): Map<number, SocialPerson> {
  const all = [
    ...initial.likes.likers,
    ...initial.comments.items.flatMap((c) => (c.reply_to ? [c.author, c.reply_to] : [c.author])),
    ...(viewer ? [viewer] : []),
  ];
  return new Map(all.map((p) => [p.id, p]));
}

/** 推送里的人：页面上没有名片的先批量取一次。 */
async function resolve(people: Map<number, SocialPerson>, ids: number[]) {
  const missing = [...new Set(ids)].filter((id) => !people.has(id));
  for (const p of await fetchPeople(missing)) people.set(p.id, p);
  return (id: number) => people.get(id) ?? { id, username: null };
}

async function socialEvent(
  e: TopicSocialEvent,
  people: Map<number, SocialPerson>,
): Promise<SocialEvent> {
  switch (e.type) {
    case "likes": {
      const who = await resolve(people, [e.user_id]);
      return { type: "likes", count: e.likes, person: who(e.user_id), liked: e.liked };
    }
    case "comment": {
      const c = e.comment;
      const who = await resolve(
        people,
        c.reply_to_id != null ? [c.author_id, c.reply_to_id] : [c.author_id],
      );
      return { type: "comment", comment: commentView(c, who), total: e.comments };
    }
    case "comment_removed":
      return { type: "comment_removed", comment_id: e.comment_id, total: e.comments };
  }
}

/**
 * 一层楼的点赞与评论状态：乐观更新，失败回滚并提示；登录后经 WebSocket 收这个主题的评论与点赞推送。
 * 设计稿模式下只改本地状态、不请求接口。`viewer` 为空（游客）时只读、不连推送。
 */
export function useFloorSocial({
  topicId,
  postId,
  initial,
  viewer,
}: {
  topicId: number;
  postId: number;
  initial: PostSocial;
  viewer: SocialPerson | null;
}) {
  const preview = usePreview();
  const [state, dispatch] = useReducer(socialReducer, initial, initialSocial);
  const [target, setTarget] = useState<Target | null>(null);
  const [notice, setNotice] = useState("");
  const opener = useRef<HTMLElement | null>(null);
  // 名字簿：推送、翻页带来的新名字也记在这里（只增不减，不触发重渲染）
  const [people] = useState(() => knownPeople(initial, viewer));
  const viewerId = viewer?.id;

  useEffect(() => {
    if (preview || viewerId === undefined) return;
    return watchTopicSocial(topicId, (e) => {
      const post = e.type === "comment" ? e.comment.post_id : e.post_id;
      if (post !== postId) return;
      const self = e.type === "likes" && e.user_id === viewerId;
      void socialEvent(e, people).then((event) => dispatch({ type: "event", event, self }));
    });
  }, [preview, topicId, postId, viewerId, people]);

  const toggleLike = () => {
    if (!viewer) return;
    const liked = !state.liked;
    dispatch({ type: "like", liked, viewer });
    if (preview) return;
    setLike(postId, liked).then(
      (r) =>
        dispatch({
          type: "event",
          event: { type: "likes", count: r.likes, person: viewer, liked: r.liked },
          self: true,
        }),
      (e: unknown) => {
        dispatch({ type: "like", liked: !liked, viewer });
        setNotice(errorMessage(e));
      },
    );
  };

  /** 打开评论框；关闭时焦点回到打开它的按钮或评论 */
  const startReply = (t: Target) => {
    if (!viewer) return;
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setTarget(t);
  };
  const closeReply = () => {
    setTarget(null);
    opener.current?.focus();
  };

  return {
    state,
    dispatch,
    people,
    target,
    startReply,
    closeReply,
    notice,
    setNotice,
    toggleLike,
    preview,
  };
}

type Social = ReturnType<typeof useFloorSocial>;

/**
 * 楼层下方的灰底评论块（朋友圈式）：先是点赞名单，再是平铺评论，最后是评论输入框。
 * 只有两级：楼层 → 平铺评论；回复某条评论只是在前面加「回复 某人」。点评论即回复评论者，
 * 长按（触屏）、右键或「…」按钮打开评论菜单。赞与评论都没有时整块不显示。
 */
export function FloorSocialBlock({
  social: s,
  postId,
  floor,
  viewer,
  canModerate,
}: {
  social: Social;
  postId: number;
  floor: number;
  viewer: SocialPerson | null;
  canModerate: boolean;
}) {
  const { state, dispatch, target, startReply, closeReply, notice, setNotice, preview } = s;
  const [expanded, setExpanded] = useState(false);
  const [loading, setLoading] = useState(false);
  const localId = useRef(-1);

  const shownLikers = state.likers.slice(0, MAX_LIKERS);
  const shownComments = expanded ? state.comments : state.comments.slice(0, VISIBLE_COMMENTS);
  const hiddenCount = state.total - shownComments.filter((c) => !c.removed).length;

  const expand = async () => {
    setExpanded(true);
    const book = s.people;
    if (preview || state.cursor === null) return;
    setLoading(true);
    try {
      let cursor: number | null = state.cursor;
      // 一次拉完：楼中评论通常不多，接口每页 50 条
      while (cursor !== null) {
        const page = await listComments(postId, cursor);
        for (const u of page.users) book.set(u.id, socialPerson(u));
        const who = (id: number) => book.get(id) ?? { id, username: null };
        cursor = page.next_cursor ?? null;
        dispatch({ type: "page", items: page.comments.map((c) => commentView(c, who)), cursor });
      }
    } catch (e) {
      setNotice(errorMessage(e));
    } finally {
      setLoading(false);
    }
  };

  const submit = async (text: string, captcha: string | null): Promise<void> => {
    if (!target || !viewer) return;
    const replyTo = target.kind === "comment" ? target.comment : null;
    try {
      if (preview) {
        const comment: PostComment = {
          id: localId.current--,
          author: viewer,
          reply_to: replyTo?.author ?? null,
          body: text,
          created_at: new Date().toISOString(),
        };
        dispatch({ type: "added", comment, total: state.total + 1 });
      } else {
        const created = await createComment(postId, text, replyTo?.id ?? null, captcha);
        const who = (id: number) =>
          id === viewer.id ? viewer : (s.people.get(id) ?? { id, username: null });
        dispatch({
          type: "added",
          comment: commentView(created.comment, who),
          total: created.comment_count,
        });
      }
      setExpanded(true);
      closeReply();
    } catch (err) {
      setNotice(errorMessage(err));
    }
  };

  /** 删除；`reason` 非空 = 管理人员删除别人的评论（留一行可以恢复）。失败时抛给理由弹窗显示。 */
  const remove = async (c: PostComment, reason: string | null) => {
    if (!preview) await deleteComment(c.id, reason);
    dispatch({ type: "removed", id: c.id, keep: reason !== null });
  };

  const restore = async (c: PostComment, reason: string) => {
    if (!preview) await restoreComment(c.id, reason);
    dispatch({ type: "restored", id: c.id });
  };

  const report = async (c: PostComment) => {
    try {
      if (!preview) await reportComment(c.id);
      setNotice(REPORTED);
    } catch (e) {
      setNotice(errorMessage(e));
    }
  };

  return (
    <>
      {state.count > 0 || state.comments.length > 0 || state.total > 0 ? (
        <div className="mt-2 rounded-md bg-wash text-sm leading-relaxed" data-testid="floor-social">
          {state.count > 0 ? (
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
                {state.count > shownLikers.length ? (
                  <span className="text-fg-muted">
                    {shownLikers.length ? " " : ""}等 {state.count} 人
                  </span>
                ) : null}
              </span>
            </p>
          ) : null}
          {state.comments.length > 0 || state.total > 0 ? (
            <ul
              aria-label={`${state.total} 条评论`}
              className={state.count > 0 ? "border-t border-border px-3 py-1.5" : "px-3 py-1.5"}
            >
              {shownComments.map((c) => (
                <CommentItem
                  key={c.id}
                  comment={c}
                  viewer={viewer}
                  canModerate={canModerate}
                  onReply={() => startReply({ kind: "comment", comment: c })}
                  onDelete={(reason) => remove(c, reason)}
                  onRestore={(reason) => restore(c, reason)}
                  onReport={() => void report(c)}
                  onNotice={setNotice}
                />
              ))}
              {hiddenCount > 0 ? (
                <li>
                  <button
                    type="button"
                    disabled={loading}
                    className="rounded-sm py-0.5 font-medium text-accent hover:underline focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-60"
                    onClick={() => void expand()}
                  >
                    {loading ? "正在加载…" : `展开其余 ${hiddenCount} 条`}
                  </button>
                </li>
              ) : null}
            </ul>
          ) : null}
        </div>
      ) : null}

      {target ? (
        <CommentForm
          // 换了回复对象就是一个新的输入框
          key={target.kind === "comment" ? target.comment.id : "floor"}
          target={target}
          floor={floor}
          onSubmit={submit}
          onClose={closeReply}
        />
      ) : null}
      <p role="status" className="text-xs text-fg-muted empty:hidden">
        {notice}
      </p>
    </>
  );
}

function CommentForm({
  target,
  floor,
  onSubmit,
  onClose,
}: {
  target: Target;
  floor: number;
  /** `captcha`：新用户的 Turnstile 令牌（不需要时为 null） */
  onSubmit: (text: string, captcha: string | null) => Promise<void>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const captcha = usePostCaptcha();
  const to = target.kind === "comment" ? userName(target.comment.author) : null;
  const send = async (e: FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text) return;
    setSending(true);
    await onSubmit(text, captcha.token);
    // Turnstile 令牌只能用一次
    captcha.renew();
    setSending(false);
  };
  return (
    <form className="mt-2 space-y-2" onSubmit={(e) => void send(e)}>
      {captcha.widget}
      <div className="flex items-center gap-2">
        <Input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") onClose();
          }}
          aria-label={to ? `回复 ${to}` : `评论 #${floor}`}
          placeholder={to ? `回复 ${to}：` : "评论…"}
          maxLength={MAX_COMMENT}
          className="h-9"
        />
        <Button type="submit" size="sm" disabled={!draft.trim() || sending || captcha.missing}>
          发送
        </Button>
        <Button variant="subtle" size="sm" onClick={onClose}>
          取消
        </Button>
      </div>
    </form>
  );
}

/** 操作行里的「赞」按钮（游客不显示）。 */
export function LikeButton({ social }: { social: Social }) {
  const { state, toggleLike } = social;
  return (
    <Button variant="subtle" size="sm" aria-pressed={state.liked} onClick={toggleLike}>
      <Heart size={16} aria-hidden fill={state.liked ? "currentColor" : "none"} />
      {state.count > 0 ? <span className="amount">{state.count}</span> : null}
      {state.liked ? "已赞" : "赞"}
    </Button>
  );
}

/** 管理人员刚删除的评论：留一行，可以带理由恢复。 */
function RemovedComment({
  comment: c,
  onRestore,
}: {
  comment: CommentEntry;
  onRestore: (reason: string) => Promise<void>;
}) {
  const [confirm, setConfirm] = useState(false);
  const name = userName(c.author);
  return (
    <li
      className="flex items-center gap-2 py-0.5 text-fg-muted"
      data-testid="floor-comment-removed"
    >
      <span className="min-w-0 flex-1 truncate">已删除 {name} 的评论</span>
      <Button variant="subtle" size="sm" className="h-7" onClick={() => setConfirm(true)}>
        <RotateCcw size={14} aria-hidden />
        恢复
      </Button>
      {confirm ? (
        <Suspense fallback={null}>
          <CommentModDialog
            title={`恢复 ${name} 的评论`}
            confirmLabel="恢复"
            preset="误操作恢复"
            onConfirm={onRestore}
            onClose={() => setConfirm(false)}
          />
        </Suspense>
      ) : null}
    </li>
  );
}

function CommentItem({
  comment: c,
  viewer,
  canModerate,
  onReply,
  onDelete,
  onRestore,
  onReport,
  onNotice,
}: {
  comment: CommentEntry;
  viewer: SocialPerson | null;
  canModerate: boolean;
  onReply: () => void;
  /** `reason` 为空 = 删除自己的评论 */
  onDelete: (reason: string | null) => Promise<void>;
  onRestore: (reason: string) => Promise<void>;
  onReport: () => void;
  onNotice: (s: string) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  const longPressed = useRef(false);
  if (c.removed) return <RemovedComment comment={c} onRestore={onRestore} />;
  const cancel = () => window.clearTimeout(timer.current);
  const own = viewer !== null && c.author.id === viewer.id;
  const name = userName(c.author);
  const removeOwn = () => onDelete(null).catch((e: unknown) => onNotice(errorMessage(e)));

  const items: LazyMenuItem[] = [];
  if (viewer) {
    items.push({ label: `回复 ${name}`, icon: <Reply size={16} aria-hidden />, onSelect: onReply });
  }
  items.push({
    label: "复制",
    icon: <Copy size={16} aria-hidden />,
    onSelect: () => {
      void navigator.clipboard?.writeText(c.body);
      onNotice("已复制评论内容");
    },
  });
  if (viewer && !own) {
    items.push({ label: "举报", icon: <Flag size={16} aria-hidden />, onSelect: onReport });
  }
  if (own || canModerate) {
    // 删除别人的评论属于管理操作，要填理由
    items.push({
      label: own ? "删除" : "删除（管理操作）",
      icon: <Trash2 size={16} aria-hidden />,
      danger: true,
      separatorBefore: true,
      onSelect: own ? () => void removeOwn() : () => setConfirmDelete(true),
    });
  }

  return (
    <>
      <li
        className="group -mx-1.5 flex cursor-pointer items-start gap-1 rounded-sm px-1.5 [-webkit-touch-callout:none] hover:bg-fg/5"
        data-testid="floor-comment"
        onClick={(e) => {
          if (longPressed.current) {
            longPressed.current = false;
            return;
          }
          // 菜单弹层经 React 冒泡上来但不在这个 DOM 里；点名字是去主页，点「…」是菜单；其余位置都是回复
          const el = e.target as HTMLElement;
          if (!e.currentTarget.contains(el) || el.closest("a, button")) return;
          if (viewer) onReply();
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
        <p className="min-w-0 flex-1 py-0.5 break-words">
          <NameLink person={c.author} />
          {c.reply_to ? (
            <>
              <span className="text-fg-muted"> 回复 </span>
              <NameLink person={c.reply_to} />
            </>
          ) : null}
          <span className="text-fg-muted">：</span>
          {c.body}
        </p>
        <LazyMenu
          align="end"
          open={menuOpen}
          onOpenChange={setMenuOpen}
          items={items}
          trigger={
            <button
              type="button"
              aria-label={`${name} 的评论：更多操作`}
              className="mt-0.5 inline-flex size-6 shrink-0 items-center justify-center rounded-sm text-fg-muted opacity-0 group-hover:opacity-100 hover:bg-surface-2 focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent data-[state=open]:opacity-100 [@media(hover:none)]:opacity-100"
            >
              <MoreHorizontal size={16} aria-hidden />
            </button>
          }
        />
      </li>
      {confirmDelete ? (
        <Suspense fallback={null}>
          <CommentModDialog
            title={`删除 ${name} 的评论`}
            confirmLabel="删除"
            danger
            onConfirm={(reason) => onDelete(reason)}
            onClose={() => setConfirmDelete(false)}
          />
        </Suspense>
      ) : null}
    </>
  );
}
