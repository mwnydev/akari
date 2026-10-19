// 模拟数据 → 正式组件的显示模型（与 lib/topic-views.ts 对接口数据做的换算同一套类型）。
import type { Schemas } from "@akari/api-client";
import type { ReactNode } from "react";
import type { DirectoryBoard } from "../../../../components/forum/BoardDirectory";
import type { BoardRef } from "../../../../components/forum/BoardMark";
import type { NavBoard, NavShortcut } from "../../../../components/forum/BoardNav";
import type { FeedRow } from "../../../../components/forum/TopicFeed";
import type { TopicHeaderView } from "../../../../components/forum/TopicHeader";
import { TopicRow, type TopicRowView } from "../../../../components/forum/TopicTable";
import { TradeLine } from "../../../../components/forum/TradeLine";
import type { PersonCard } from "../../../../components/forum/UserPopover";
import type { PostSocial, SocialPerson } from "../../../../lib/floor-social";
import { PROSE_CLASS } from "../../../../lib/prose";
import { AuctionLine } from "../../../../components/forum/AuctionLine";
import {
  board as boardOf,
  boards,
  user,
  VIEWER_ID,
  type Board,
  type FloorData,
  type TopicSummary,
} from "./data";

/** 设计稿里只有交易板块有单独的页面。 */
function boardHref(slug: string): string {
  return slug === "trade" ? "/design/v2/board" : "#";
}

export function mockBoardRef(b: Board): BoardRef {
  return { name: b.name, color: b.color, href: boardHref(b.slug) };
}

export const mockShortcuts: NavShortcut[] = [
  { key: "home", href: "/design/v2", label: "最新", icon: "latest" },
  { key: "hot", href: "/design/v2", label: "热门", icon: "hot" },
  { key: "bookmarks", href: "#", label: "我的收藏", icon: "bookmarks" },
];

export const mockNavBoards: NavBoard[] = boards.map((b) => ({
  slug: b.slug,
  ...mockBoardRef(b),
  today: b.today,
}));

export const mockDirectory: DirectoryBoard[] = boards.map((b) => ({
  slug: b.slug,
  ...mockBoardRef(b),
  description: b.description,
  topics: b.topics,
  today: b.today,
}));

export function mockPerson(id: number): PersonCard {
  const u = user(id);
  return {
    id: u.id,
    name: u.name,
    href: "#",
    level: u.level,
    levelName: u.levelName,
    online: u.online,
    lastSeen: u.lastSeen,
    reputation: u.reputation,
    role: u.role,
    stats: [
      ["加入", u.joined],
      ["帖子", String(u.posts)],
      ["获赞", String(u.likes)],
    ],
  };
}

function social(id: number): SocialPerson {
  return { id, username: user(id).name };
}

export const mockViewer = social(VIEWER_ID);
/** 「管理」菜单的目标板块（设计稿模式下不会真的提交）。 */
export const mockForums = boards.map((b, i) => ({ id: i + 1, name: b.name }));
export const mockCanModerate = user(VIEWER_ID).role !== undefined;
/** 访问者是版主时，「管理」菜单按 1 号板块的版主权限列出（设计稿模式下不会真的提交）。 */
export const mockStaff: Schemas["StaffView"] | null = mockCanModerate
  ? {
      admin_backend: false,
      roles: ["moderator"],
      boards: [1, 2],
      board_permissions: ["topic_moderate", "topic_hide", "post_delete"],
      site_permissions: [],
      suspended: false,
    }
  : null;

function rowView(t: TopicSummary): TopicRowView {
  return {
    id: t.id,
    title: t.title,
    href: t.href,
    kind: t.trade ? "trade" : t.auction ? "auction" : "normal",
    board: mockBoardRef(boardOf(t.board)),
    author: mockPerson(t.authorId),
    participants: t.participants.map((id) => ({ name: user(id).name })),
    replies: t.replies,
    views: t.views,
    activityIso: t.activityIso,
    activity: t.activity,
    pinned: t.pinned ?? false,
    featured: t.featured ?? false,
    locked: t.locked ?? false,
    hidden: false,
    unread: t.unread,
  };
}

function info(t: TopicSummary): ReactNode {
  if (t.trade) return <TradeLine trade={t.trade} />;
  if (t.auction) return <AuctionLine auction={t.auction} />;
  return null;
}

export function mockRows(list: TopicSummary[]): FeedRow[] {
  return list.map((t) => {
    const view = rowView(t);
    return {
      id: t.id,
      kind: view.kind,
      pinned: view.pinned,
      node: <TopicRow key={t.id} topic={view} info={info(t)} />,
    };
  });
}

export function mockHeader(t: TopicSummary): TopicHeaderView {
  return {
    title: t.title,
    kind: t.trade ? "trade" : t.auction ? "auction" : "normal",
    board: mockBoardRef(boardOf(t.board)),
    pinned: t.pinned ?? false,
    locked: t.locked ?? false,
    featured: t.featured ?? false,
    hidden: false,
    replies: t.replies,
    views: t.views,
    participants: t.participants.length,
  };
}

/** 楼层的赞与评论：模拟数据里评论一次全给，前 5 条之后由「展开其余」在本地展开。 */
export function mockSocial(f: FloorData): PostSocial {
  return {
    likes: {
      count: f.likeCount,
      likers: f.likers.map(social),
      liked: f.likers.includes(VIEWER_ID),
    },
    comments: {
      total: f.comments.length,
      items: f.comments.map((c) => ({
        id: c.id,
        author: social(c.authorId),
        reply_to: c.replyTo ? social(c.replyTo) : null,
        body: c.text,
        created_at: "",
      })),
      next_cursor: null,
    },
  };
}

/** 楼层正文：与正式页面同样的排版；引用块的写法与「引用回复」插入编辑器的一致。 */
export function mockBody(f: FloorData): ReactNode {
  return (
    <div className={PROSE_CLASS}>
      {f.quote ? (
        <blockquote>
          <p>
            <a href={`#floor-${f.quote.floor}`}>
              #{f.quote.floor} {user(f.quote.authorId).name}
            </a>
            ：{f.quote.text}
          </p>
        </blockquote>
      ) : null}
      {f.body.map((p) => (
        <p key={p}>{p}</p>
      ))}
    </div>
  );
}
