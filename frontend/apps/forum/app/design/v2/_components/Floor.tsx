import { Avatar, Badge } from "@akari/ui";
import type { ReactNode } from "react";
import { PROSE_CLASS } from "../../../../lib/prose";
import { user, VIEWER_ID, type FloorData } from "../_mock/data";
import { FloorSocial, type Person } from "./FloorSocial";
import { UserPopover } from "./UserPopover";

function person(id: number): Person {
  return { id, name: user(id).name };
}

/**
 * 一层楼（Discourse 式：左侧头像、右侧正文，楼层之间 1px 分隔线，没有卡片；窄屏头像收进标题行）。
 * 头像和名字都能打开用户卡片；正文下方是朋友圈式的赞与评论。`extra` 放嵌入正文的内容（报价卡片等）。
 */
export function Floor({
  data: f,
  opAuthorId,
  extra,
}: {
  data: FloorData;
  /** 楼主，名字旁标「楼主」 */
  opAuthorId: number;
  extra?: ReactNode;
}) {
  const author = user(f.authorId);
  const viewer = user(VIEWER_ID);
  return (
    <article
      id={`floor-${f.floor}`}
      data-floor={f.floor}
      aria-label={`#${f.floor} ${author.name}`}
      className="flex scroll-mt-20 gap-3 border-t border-border py-4 sm:gap-4"
    >
      <div className="hidden sm:block">
        <UserPopover user={author}>
          <Avatar name={author.name} size={45} online={author.online} />
        </UserPopover>
      </div>
      <div className="min-w-0 flex-1">
        <header className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
          <span className="mr-0.5 sm:hidden">
            <UserPopover user={author}>
              <Avatar name={author.name} size={32} online={author.online} />
            </UserPopover>
          </span>
          <UserPopover
            user={author}
            label={`${author.name}，查看名片`}
            className="rounded-sm font-semibold hover:underline focus-visible:outline-2 focus-visible:outline-accent"
          >
            {author.name}
          </UserPopover>
          <Badge tone="accent">Lv{author.level}</Badge>
          {author.role ? <Badge>{author.role}</Badge> : null}
          {f.authorId === opAuthorId ? <Badge tone="outline">楼主</Badge> : null}
          <span className="ml-auto flex items-center gap-2 text-sm text-fg-muted">
            {f.edited ? <span>已编辑</span> : null}
            <time dateTime={f.timeIso}>{f.time}</time>
            <a href={`#floor-${f.floor}`} className="amount rounded-sm hover:text-fg">
              #{f.floor}
            </a>
          </span>
        </header>
        {f.quote ? (
          <blockquote className="mt-2 border-l-4 border-border bg-wash px-3 py-2 text-sm">
            <a href={`#floor-${f.quote.floor}`} className="text-xs text-fg-muted hover:text-fg">
              引用 #{f.quote.floor} {user(f.quote.authorId).name}
            </a>
            <p className="text-fg-muted">{f.quote.text}</p>
          </blockquote>
        ) : null}
        <div className={`${PROSE_CLASS} mt-2`}>
          {f.body.map((p) => (
            <p key={p}>{p}</p>
          ))}
        </div>
        {extra ? <div className="mt-3">{extra}</div> : null}
        <FloorSocial
          floor={f.floor}
          floorAuthor={author.name}
          likers={f.likers.map(person)}
          likeCount={f.likeCount}
          comments={f.comments.map((c) => ({
            id: c.id,
            author: person(c.authorId),
            ...(c.replyTo ? { replyTo: person(c.replyTo) } : {}),
            text: c.text,
          }))}
          viewer={person(viewer.id)}
          canModerate={viewer.role !== undefined}
        />
      </div>
    </article>
  );
}

/** 楼层流：没有外框，楼层之间是分隔线。 */
export function FloorStream({ children, label }: { children: ReactNode; label: string }) {
  return (
    <section aria-label={label} className="border-b border-border">
      {children}
    </section>
  );
}
