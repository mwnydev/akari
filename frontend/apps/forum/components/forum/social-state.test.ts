import { describe, expect, it } from "vitest";
import type { PostComment } from "../../lib/floor-social";
import { initialSocial, socialReducer, type SocialState } from "./social-state";

const me = { id: 1, username: "me" };
const other = { id: 2, username: "other" };
const comment = (id: number): PostComment => ({
  id,
  author: other,
  reply_to: null,
  body: `c${id}`,
  created_at: "",
});

const base: SocialState = initialSocial({
  likes: { count: 1, likers: [other], liked: false },
  comments: { total: 7, items: [comment(1), comment(2)], next_cursor: 2 },
});

describe("floor social state", () => {
  it("likes put the viewer first and unlikes take them out", () => {
    const liked = socialReducer(base, { type: "like", liked: true, viewer: me });
    expect(liked).toMatchObject({ liked: true, count: 2, likers: [me, other] });
    expect(socialReducer(liked, { type: "like", liked: true, viewer: me })).toBe(liked);
    expect(socialReducer(liked, { type: "like", liked: false, viewer: me })).toMatchObject({
      liked: false,
      count: 1,
      likers: [other],
    });
  });

  it("takes the like count from pushes and only flips `liked` for the viewer's own", () => {
    const pushed = socialReducer(base, {
      type: "event",
      event: { type: "likes", count: 5, person: { id: 3, username: "c" }, liked: true },
      self: false,
    });
    expect(pushed).toMatchObject({ count: 5, liked: false });
    expect(pushed.likers.map((p) => p.id)).toEqual([3, 2]);
    const mine = socialReducer(pushed, {
      type: "event",
      event: { type: "likes", count: 6, person: me, liked: true },
      self: true,
    });
    expect(mine).toMatchObject({ count: 6, liked: true });
  });

  it("adds comments once and keeps the server's total", () => {
    const added = socialReducer(base, { type: "added", comment: comment(9), total: 8 });
    expect(added.total).toBe(8);
    // 推送回来的同一条评论不重复
    const echoed = socialReducer(added, {
      type: "event",
      event: { type: "comment", comment: comment(9), total: 8 },
      self: false,
    });
    expect(echoed.comments.map((c) => c.id)).toEqual([1, 2, 9]);
    const removed = socialReducer(echoed, {
      type: "event",
      event: { type: "comment_removed", comment_id: 1, total: 7 },
      self: false,
    });
    expect(removed.comments.map((c) => c.id)).toEqual([2, 9]);
    expect(removed.total).toBe(7);
  });

  it("keeps a staff-deleted comment as a restorable row until restored", () => {
    const removed = socialReducer(base, { type: "removed", id: 1, keep: true });
    expect(removed.comments[0]).toMatchObject({ id: 1, removed: true });
    expect(removed.total).toBe(6);
    // 自己删除引起的推送不收起这一行
    const echoed = socialReducer(removed, {
      type: "event",
      event: { type: "comment_removed", comment_id: 1, total: 6 },
      self: false,
    });
    expect(echoed.comments[0]).toMatchObject({ id: 1, removed: true });
    const restored = socialReducer(echoed, { type: "restored", id: 1 });
    expect(restored.comments[0]).toMatchObject({ id: 1, removed: false });
    expect(restored.total).toBe(7);
    // 作者自己删除：直接去掉
    expect(socialReducer(base, { type: "removed", id: 2, keep: false }).comments).toHaveLength(1);
  });

  it("merges later pages without duplicates", () => {
    const paged = socialReducer(base, {
      type: "page",
      items: [comment(2), comment(3)],
      cursor: null,
    });
    expect(paged.comments.map((c) => c.id)).toEqual([1, 2, 3]);
    expect(paged).toMatchObject({ cursor: null, total: 7 });
  });
});
