import type { Schemas } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { commentView, peopleOf, socialView } from "./floor-social";

const people = peopleOf([
  { id: 1, username: "楼主" },
  { id: 2, username: null },
]);

const comment = (id: number, reply_to_id: number | null = null): Schemas["FloorComment"] => ({
  id,
  post_id: 9,
  author_id: 2,
  reply_to_id,
  body: "<b>纯文本</b>",
  created_at: "2026-10-20T00:00:00Z",
});

describe("floor social views", () => {
  it("resolves comment authors and the replied-to person from the page's cards", () => {
    expect(commentView(comment(5, 1), people)).toEqual({
      id: 5,
      author: { id: 2, username: null },
      reply_to: { id: 1, username: "楼主" },
      body: "<b>纯文本</b>",
      created_at: "2026-10-20T00:00:00Z",
    });
    // 不在名片里的人按 ID 显示
    expect(commentView({ ...comment(6), author_id: 77 }, people).author).toEqual({
      id: 77,
      username: null,
    });
  });

  it("pages from the last preview comment only when more remain", () => {
    const base: Schemas["FloorSocialView"] = {
      post_id: 9,
      likes: 3,
      liked: true,
      likers: [1, 2],
      comment_count: 2,
      comments: [comment(4), comment(8)],
    };
    const all = socialView(base, people);
    expect(all.likes).toEqual({
      count: 3,
      liked: true,
      likers: [
        { id: 1, username: "楼主" },
        { id: 2, username: null },
      ],
    });
    expect(all.comments).toMatchObject({ total: 2, next_cursor: null });
    expect(socialView({ ...base, comment_count: 12 }, people).comments.next_cursor).toBe(8);
  });
});
