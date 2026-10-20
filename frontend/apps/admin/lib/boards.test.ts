import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { boardPatch, boardValues, PALETTE, type Board } from "./boards";

const board: Board = {
  id: 3,
  parent_id: null,
  slug: "trade",
  name: "交易区",
  description: "担保交易帖",
  position: 3,
  view_min_level: 0,
  post_min_level: 1,
  reply_min_level: 1,
  allow_trade: true,
  allow_auction: false,
  color: "#2e9d5c",
};

describe("boardPatch", () => {
  it("sends only changed fields", () => {
    expect(boardPatch(board, boardValues(board))).toBeNull();
    expect(boardPatch(board, { ...boardValues(board), name: " 交易 ", position: 1 })).toEqual({
      body: { name: "交易", position: 1 },
      moneyChanged: false,
    });
  });

  it("flags auction changes for re-verification", () => {
    expect(boardPatch(board, { ...boardValues(board), allowAuction: true })).toEqual({
      body: { allow_auction: true },
      moneyChanged: true,
    });
  });

  it("clears the colour with null and flags trade changes for re-verification", () => {
    expect(boardPatch(board, { ...boardValues(board), color: "", allowTrade: false })).toEqual({
      body: { color: null, allow_trade: false },
      moneyChanged: true,
    });
  });
});

describe("PALETTE", () => {
  it("matches the forum's board colour tokens", () => {
    const css = readFileSync(new URL("../../forum/app/globals.css", import.meta.url), "utf8");
    const tokens = [...css.matchAll(/--color-board-[a-z]+:\s*(#[0-9a-f]{6})/g)].map((m) => m[1]);
    expect(PALETTE.map((p) => p.hex)).toEqual(tokens);
  });
});
