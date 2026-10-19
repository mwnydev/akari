import type { Schemas } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { badgeText } from "./badge";
import {
  applyMessage,
  applyRead,
  chainBreak,
  lastId,
  mergeById,
  mergeNewestFirst,
  readTarget,
} from "./chat";

const ME = 1;
const PEER = 2;

function msg(id: number, conversation_id: number, sender_id: number | null = PEER) {
  return {
    id,
    conversation_id,
    sender_id,
    kind: "text",
    body: `m${id}`,
    warn: false,
    created_at: "2026-10-19T00:00:00Z",
  } satisfies Schemas["Message"];
}

function conv(id: number, last: number, unread = 0, my_read_id = 0): Schemas["ConversationView"] {
  return {
    id,
    peer: {
      id: PEER,
      level: 1,
      reputation: { success: 0, failed: 0, settled: 0, disputes_lost: 0 },
    },
    last_message: msg(last, id),
    unread,
    my_read_id,
    peer_read_id: 0,
    blocked: false,
  };
}

describe("mergeById", () => {
  it("dedupes overlapping pages and pushes, keeping time order", () => {
    const page = [msg(3, 1), msg(4, 1)];
    const merged = mergeById(page, [msg(5, 1), msg(4, 1), msg(1, 1)]);
    expect(merged.map((m) => m.id)).toEqual([1, 3, 4, 5]);
    expect(mergeById(page, [])).toBe(page);
  });

  it("newest-first lists stay descending", () => {
    expect(mergeNewestFirst([{ id: 9 }, { id: 7 }], [{ id: 8 }, { id: 9 }])).toEqual([
      { id: 9 },
      { id: 8 },
      { id: 7 },
    ]);
  });

  it("lastId is 0 for an empty list", () => {
    expect(lastId([])).toBe(0);
    expect(lastId([{ id: 4 }, { id: 9 }, { id: 2 }])).toBe(9);
  });
});

describe("applyMessage", () => {
  it("moves the conversation to the top and counts unread from the peer", () => {
    const list = [conv(10, 5), conv(11, 3, 2)];
    const out = applyMessage(list, msg(6, 11), ME, null) ?? [];
    expect(out.map((c) => c.id)).toEqual([11, 10]);
    expect(out[0]?.unread).toBe(3);
    expect(out[0]?.last_message?.id).toBe(6);
  });

  it("does not count my own messages or the conversation I am reading", () => {
    const list = [conv(10, 5, 1)];
    expect(applyMessage(list, msg(6, 10, ME), ME, null)?.[0]?.unread).toBe(1);
    expect(applyMessage(list, msg(7, 10), ME, 10)?.[0]?.unread).toBe(1);
  });

  it("ignores duplicates and older messages (push after a refetch)", () => {
    const list = [conv(10, 8, 1)];
    expect(applyMessage(list, msg(8, 10), ME, null)).toBe(list);
    expect(applyMessage(list, msg(7, 10), ME, null)).toBe(list);
  });

  it("caps unread at 100 like the server", () => {
    expect(applyMessage([conv(10, 5, 100)], msg(6, 10), ME, null)?.[0]?.unread).toBe(100);
  });

  it("asks for a refetch when the conversation is not in the list", () => {
    expect(applyMessage([conv(10, 5)], msg(6, 99), ME, null)).toBeNull();
  });
});

describe("applyRead", () => {
  it("clears my unread when another device of mine caught up", () => {
    const out = applyRead(
      [conv(10, 8, 3, 2)],
      { conversation_id: 10, user_id: ME, last_id: 8 },
      ME,
    );
    expect(out[0]).toMatchObject({ unread: 0, my_read_id: 8 });
  });

  it("keeps unread when the read position is behind the last message", () => {
    const out = applyRead(
      [conv(10, 8, 3, 2)],
      { conversation_id: 10, user_id: ME, last_id: 6 },
      ME,
    );
    expect(out[0]).toMatchObject({ unread: 3, my_read_id: 6 });
  });

  it("only moves read receipts forward", () => {
    const list = [{ ...conv(10, 8), peer_read_id: 7 }];
    expect(applyRead(list, { conversation_id: 10, user_id: PEER, last_id: 5 }, ME)[0]).toBe(
      list[0],
    );
    expect(
      applyRead(list, { conversation_id: 10, user_id: PEER, last_id: 8 }, ME)[0]?.peer_read_id,
    ).toBe(8);
  });
});

describe("readTarget", () => {
  it("reports the newest message from the peer past my read position", () => {
    const list = [msg(1, 1), msg(2, 1, ME), msg(3, 1), msg(4, 1, ME)];
    expect(readTarget(list, ME, 0)).toBe(3);
    expect(readTarget(list, ME, 3)).toBeNull();
    expect(readTarget([msg(5, 1, ME)], ME, 0)).toBeNull();
  });

  it("treats system cards (no sender) as from the peer", () => {
    expect(readTarget([msg(6, 1, null)], ME, 0)).toBe(6);
  });
});

describe("badgeText", () => {
  it("hides zero and caps at 99+", () => {
    expect(badgeText(0)).toBeNull();
    expect(badgeText(7)).toBe("7");
    expect(badgeText(100)).toBe("99+");
  });
});

describe("chainBreak", () => {
  const link = (id: number, prev: string, hash: string) => ({ id, prev_hash: prev, hash });

  it("accepts a contiguous chain (hex case-insensitive)", () => {
    expect(
      chainBreak([link(1, "0x00", "0xAA"), link(5, "0xaa", "0xbb"), link(9, "0xbb", "0xcc")]),
    ).toBeNull();
    expect(chainBreak([])).toBeNull();
  });

  it("reports the first message that does not link to its predecessor", () => {
    expect(
      chainBreak([link(1, "0x00", "0xaa"), link(2, "0xff", "0xbb"), link(3, "0xee", "0xcc")]),
    ).toBe(2);
  });
});
