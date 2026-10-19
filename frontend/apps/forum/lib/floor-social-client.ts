// 楼层点赞与评论的浏览器端请求（同源 /api/v1）与主题页推送订阅。
import type { Schemas } from "@akari/api-client";
import { api, unwrap } from "./browser-api";
import { socialPerson, type SocialPerson } from "./floor-social";
import type { RealtimeEvent } from "./realtime/events";
import { acquireHub, releaseHub } from "./realtime/react";

export async function setLike(postId: number, liked: boolean): Promise<Schemas["LikeState"]> {
  const params = { path: { id: postId } };
  return unwrap(
    liked
      ? api.PUT("/api/v1/posts/{id}/like", { params })
      : api.DELETE("/api/v1/posts/{id}/like", { params }),
  );
}

export function listComments(postId: number, after: number): Promise<Schemas["CommentPage"]> {
  return unwrap(
    api.GET("/api/v1/posts/{id}/comments", { params: { path: { id: postId }, query: { after } } }),
  );
}

export function createComment(
  postId: number,
  body: string,
  replyTo: number | null,
): Promise<Schemas["CreatedComment"]> {
  return unwrap(
    api.POST("/api/v1/posts/{id}/comments", {
      params: { path: { id: postId } },
      body: { body, reply_to: replyTo },
    }),
  );
}

/** 删除评论；删除别人的评论（管理操作）必须带理由。 */
export async function deleteComment(id: number, reason: string | null): Promise<void> {
  await unwrap(
    api.DELETE("/api/v1/comments/{id}", {
      params: { path: { id }, query: reason ? { reason } : {} },
    }),
  );
}

export async function restoreComment(id: number, reason: string): Promise<void> {
  await unwrap(
    api.POST("/api/v1/comments/{id}/restore", { params: { path: { id } }, body: { reason } }),
  );
}

export async function reportComment(id: number): Promise<void> {
  await unwrap(
    api.POST("/api/v1/comments/{id}/report", { params: { path: { id } }, body: { reason: "" } }),
  );
}

export async function reportPost(id: number): Promise<void> {
  await unwrap(
    api.POST("/api/v1/posts/{id}/report", { params: { path: { id } }, body: { reason: "" } }),
  );
}

export type TopicSocialEvent = Extract<
  RealtimeEvent,
  { type: "comment" } | { type: "comment_removed" } | { type: "likes" }
>;

/**
 * 订阅主题页的评论与点赞推送（WebSocket `{"type":"subscribe","topic_id":N}`，与私信、订单共用一条连接，
 * 同一主题被多层楼订阅时只发一次），返回退订函数。只有登录用户能连。
 */
export function watchTopicSocial(
  topicId: number,
  onEvent: (e: TopicSocialEvent) => void,
): () => void {
  const hub = acquireHub();
  const unsubscribe = hub.subscribe({ topic_id: topicId });
  const off = hub.on((e) => {
    if (
      (e.type === "comment" || e.type === "comment_removed" || e.type === "likes") &&
      e.topic_id === topicId
    ) {
      onEvent(e);
    }
  });
  return () => {
    off();
    unsubscribe();
    releaseHub();
  };
}

/** 推送里出现、页面上还没有名片的人：批量取一次名片（取不到的显示「用户 {id}」）。 */
export async function fetchPeople(ids: number[]): Promise<SocialPerson[]> {
  if (!ids.length) return [];
  try {
    const cards = await unwrap(
      api.GET("/api/v1/users/cards", { params: { query: { ids: ids.join(",") } } }),
    );
    return cards.map(socialPerson);
  } catch {
    return ids.map((id) => ({ id, username: null }));
  }
}
