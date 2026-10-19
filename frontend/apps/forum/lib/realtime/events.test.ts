import { describe, expect, it } from "vitest";
import { eventTopic, parseEvent, topicKey, topicOf } from "./events";

describe("topic page events", () => {
  it("round-trips the topic subscription key", () => {
    expect(topicKey({ topic_id: 12 })).toBe("t:12");
    expect(topicOf("t:12")).toEqual({ topic_id: 12 });
    expect(topicOf("x:12")).toBeNull();
  });

  it("parses comment, removal and like pushes and files them under the topic", () => {
    const comment = parseEvent(
      JSON.stringify({
        type: "comment",
        topic_id: 3,
        comments: 2,
        comment: { id: 5, post_id: 9, author_id: 1, body: "hi", created_at: "t" },
      }),
    );
    expect(comment && eventTopic(comment)).toBe("t:3");
    const removed = parseEvent({
      type: "comment_removed",
      topic_id: 3,
      post_id: 9,
      comment_id: 5,
      comments: 1,
    });
    expect(removed?.type).toBe("comment_removed");
    const likes = parseEvent({
      type: "likes",
      topic_id: 3,
      post_id: 9,
      user_id: 1,
      liked: true,
      likes: 4,
    });
    expect(likes?.type).toBe("likes");
    expect(
      parseEvent({ type: "comment", topic_id: 3, comments: 1, comment: { id: 5 } }),
    ).toBeNull();
    expect(parseEvent({ type: "likes", topic_id: 3, post_id: 9, user_id: 1, likes: 4 })).toBeNull();
  });

  it("files a topic subscription reply under the topic", () => {
    const e = parseEvent({ type: "subscribed", topic_id: 4, presence: [] });
    expect(e && eventTopic(e)).toBe("t:4");
  });
});

describe("auction events", () => {
  // forum-core `chat::Event::AuctionBid` / `AuctionState` 序列化后的样子
  const bid = {
    type: "auction_bid",
    topic_id: 103,
    auction_id: 9,
    bid_id: 24,
    amount: "130000",
    bidder: "夜***风",
    linked: false,
    bids: 24,
    ends_at: "2026-10-20T10:14:05Z",
    reserve_met: true,
  };

  it("parses bid and state pushes on the topic channel", () => {
    const e = parseEvent(bid);
    expect(e && eventTopic(e)).toBe("t:103");
    const state = parseEvent({
      type: "auction_state",
      topic_id: 103,
      auction_id: 9,
      state: "unsold",
    });
    expect(state?.type).toBe("auction_state");
    expect(state && eventTopic(state)).toBe("t:103");
  });

  it("drops malformed auction pushes", () => {
    expect(
      parseEvent({ type: "auction_state", topic_id: 103, auction_id: 9, state: "paid" }),
    ).toBeNull();
    expect(parseEvent({ type: "auction_state", topic_id: 103, state: "unsold" })).toBeNull();
    expect(parseEvent({ ...bid, amount: "1.3" })).toBeNull();
    expect(parseEvent({ ...bid, ends_at: "soon" })).toBeNull();
    expect(parseEvent({ ...bid, linked: "no" })).toBeNull();
  });
});
