// 楼层点赞与朋友圈式评论：接口数据 → 显示模型（纯函数，单元测试直接测）；浏览器端请求见 floor-social-client.ts。
// 评论正文是纯文本，组件按文本渲染（不经 dangerouslySetInnerHTML）。
import type { Schemas } from "@akari/api-client";
import type { UserCardView } from "./display";

/** 点赞名单、评论里出现的人（UserCardView 的子集）。 */
export interface SocialPerson {
  id: number;
  username: string | null;
}

/** 楼中评论：只有两级，回复某条评论只是带上被回复的人。 */
export interface PostComment {
  id: number;
  author: SocialPerson;
  reply_to: SocialPerson | null;
  body: string;
  created_at: string;
}

export interface PostSocial {
  /** 点赞：总数、最近几位、访问者自己是否赞过（游客恒为 false） */
  likes: { count: number; likers: SocialPerson[]; liked: boolean };
  /** 评论：随楼层带回的前几条 + 总数；`next_cursor` 是「展开其余」的 `after` 游标 */
  comments: { total: number; items: PostComment[]; next_cursor: number | null };
}

/** 主题页推送（`subscribe` 了 `topic_id` 之后）换算成的楼层事件。 */
export type SocialEvent =
  | { type: "likes"; count: number; person: SocialPerson; liked: boolean }
  | { type: "comment"; comment: PostComment; total: number }
  | { type: "comment_removed"; comment_id: number; total: number };

export type People = (id: number) => SocialPerson;

export function socialPerson(u: Pick<UserCardView, "id" | "username">): SocialPerson {
  return { id: u.id, username: u.username ?? null };
}

/** 按 `users` 查人；不在里面的显示「用户 {id}」。 */
export function peopleOf(users: Pick<UserCardView, "id" | "username">[]): People {
  const byId = new Map(users.map((u) => [u.id, socialPerson(u)]));
  return (id) => byId.get(id) ?? { id, username: null };
}

export function commentView(c: Schemas["FloorComment"], people: People): PostComment {
  return {
    id: c.id,
    author: people(c.author_id),
    reply_to: c.reply_to_id != null ? people(c.reply_to_id) : null,
    body: c.body,
    created_at: c.created_at,
  };
}

/** 评论分页游标：还有没拉下来的就是已有的最后一条评论 ID（接口按 ID 正序翻页）。 */
function nextCursor(items: { id: number }[], total: number): number | null {
  return total > items.length ? (items.at(-1)?.id ?? null) : null;
}

export function socialView(s: Schemas["FloorSocialView"], people: People): PostSocial {
  const items = s.comments.map((c) => commentView(c, people));
  return {
    likes: { count: s.likes, likers: s.likers.map(people), liked: s.liked },
    comments: {
      total: s.comment_count,
      items,
      next_cursor: nextCursor(items, s.comment_count),
    },
  };
}
