// 楼层点赞与评论的状态变化（纯函数，FloorSocial 用 useReducer 驱动，单元测试直接测）。
import type { PostComment, PostSocial, SocialEvent, SocialPerson } from "../../lib/floor-social";

/** 列表里的一条评论；`removed` = 访问者刚以管理人员身份删除，留一行给「恢复」。 */
export interface CommentEntry extends PostComment {
  removed?: boolean;
}

export interface SocialState {
  count: number;
  likers: SocialPerson[];
  liked: boolean;
  comments: CommentEntry[];
  /** 未删除的评论总数（含还没拉下来的），以服务端给的数为准 */
  total: number;
  /** 「展开其余」的 `after` 游标；null = 已全部拉下 */
  cursor: number | null;
}

export type SocialAction =
  | { type: "like"; liked: boolean; viewer: SocialPerson }
  | { type: "added"; comment: PostComment; total: number }
  /** `keep`：管理人员删除别人的评论，留一行可以恢复 */
  | { type: "removed"; id: number; keep: boolean }
  | { type: "restored"; id: number }
  | { type: "page"; items: PostComment[]; cursor: number | null }
  /** 主题页推送；`self` = 推送里的人就是访问者 */
  | { type: "event"; event: SocialEvent; self: boolean };

/** 点赞名单只留、只显示最近几位（与接口一致），其余显示「等 N 人」。 */
export const MAX_LIKERS = 5;

export function initialSocial(s: PostSocial): SocialState {
  return {
    count: s.likes.count,
    likers: s.likes.likers,
    liked: s.likes.liked,
    comments: s.comments.items,
    total: s.comments.total,
    cursor: s.comments.next_cursor,
  };
}

function withLiker(s: SocialState, p: SocialPerson, liked: boolean): SocialPerson[] {
  const others = s.likers.filter((x) => x.id !== p.id);
  return liked ? [p, ...others].slice(0, MAX_LIKERS) : others;
}

function addComment(s: SocialState, c: CommentEntry, total: number): SocialState {
  const exists = s.comments.some((x) => x.id === c.id);
  return {
    ...s,
    comments: exists
      ? s.comments.map((x) => (x.id === c.id ? { ...c, removed: false } : x))
      : [...s.comments, c],
    total,
  };
}

function removeComment(s: SocialState, id: number, keep: boolean, total: number): SocialState {
  const hit = s.comments.find((x) => x.id === id);
  if (!hit) return { ...s, total };
  // 本页刚删、留着「恢复」的那一行，推送回来时不收起
  const comments =
    keep || hit.removed
      ? s.comments.map((x) => (x.id === id ? { ...x, removed: true } : x))
      : s.comments.filter((x) => x.id !== id);
  return { ...s, comments, total };
}

export function socialReducer(s: SocialState, a: SocialAction): SocialState {
  switch (a.type) {
    case "like":
      if (a.liked === s.liked) return s;
      return {
        ...s,
        liked: a.liked,
        count: Math.max(0, s.count + (a.liked ? 1 : -1)),
        likers: withLiker(s, a.viewer, a.liked),
      };
    case "added":
      return addComment(s, a.comment, a.total);
    case "removed": {
      const live = s.comments.some((x) => x.id === a.id && !x.removed);
      return removeComment(s, a.id, a.keep, live ? Math.max(0, s.total - 1) : s.total);
    }
    case "restored": {
      const hit = s.comments.find((x) => x.id === a.id && x.removed);
      if (!hit) return s;
      return addComment(s, { ...hit, removed: false }, s.total + 1);
    }
    case "page": {
      const seen = new Set(s.comments.map((c) => c.id));
      return {
        ...s,
        comments: [...s.comments, ...a.items.filter((c) => !seen.has(c.id))],
        cursor: a.cursor,
      };
    }
    case "event": {
      const e = a.event;
      switch (e.type) {
        case "likes":
          return {
            ...s,
            count: e.count,
            likers: withLiker(s, e.person, e.liked),
            liked: a.self ? e.liked : s.liked,
          };
        // 自己刚发的评论推送回来时按 id 去重；还没展开全部时也先追加，马上能看到新评论
        case "comment":
          return addComment(s, e.comment, e.total);
        case "comment_removed":
          return removeComment(s, e.comment_id, false, e.total);
      }
    }
  }
}
