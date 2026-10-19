import { describe, expect, it, vi } from "vitest";

vi.mock("server-only", () => ({}));
const { forumTree } = await import("./forums");

const f = (id: number, parent_id: number | null, position = id) => ({
  id,
  parent_id,
  position,
  slug: `f${id}`,
  name: `F${id}`,
  description: "",
  view_min_level: 0,
  post_min_level: 1,
  reply_min_level: 1,
  allow_trade: false,
});

describe("forumTree", () => {
  it("groups children under their top-level ancestor in position order", () => {
    const tree = forumTree([f(3, 1, 2), f(1, null, 1), f(2, null, 0), f(4, 3, 0), f(5, 1, 1)]);
    expect(tree.map((n) => [n.id, n.children.map((c) => c.id)])).toEqual([
      [2, []],
      [1, [4, 5, 3]],
    ]);
  });

  it("treats a forum whose parent is invisible as top-level", () => {
    expect(forumTree([f(7, 99)]).map((n) => n.id)).toEqual([7]);
  });
});
