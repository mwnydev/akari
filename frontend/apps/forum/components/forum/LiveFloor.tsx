import type { Me, Schemas } from "@akari/api-client";
import {
  cardLookup,
  formatActivity,
  formatTime,
  userName,
  type UserCardView,
} from "../../lib/display";
import { peopleOf, socialView, type PostSocial, type SocialPerson } from "../../lib/floor-social";
import { topicHref } from "../../lib/links";
import { can } from "../../lib/staff";
import { personCard } from "../../lib/topic-views";
import { PresenceDot } from "../live/PresenceDot";
import type { PostModeration } from "../moderation/usePostModeration";
import { Prose } from "../Prose";
import { Floor } from "./Floor";
import { FloorContent, type FloorPermissions } from "./FloorContent";

/**
 * 接口里的一层楼 → 共享的 Floor + FloorContent（主题页与交易帖共用）。
 * `social` 是这层楼的点赞与评论（已删除的楼层为空）；`canModerate` = 能删除别人的评论；`mod` 只给管理人员。
 */
export function LiveFloor({
  post,
  author,
  topic,
  page,
  perms,
  mentionable,
  social,
  viewer,
  canModerate,
  canReply,
  mod,
  now,
}: {
  post: Schemas["PostView"];
  author: UserCardView;
  topic: Schemas["TopicSummary"];
  page: number;
  perms: FloorPermissions;
  mentionable: UserCardView[];
  social: PostSocial | null;
  viewer: SocialPerson | null;
  canModerate: boolean;
  canReply: boolean;
  mod?: PostModeration | undefined;
  now: Date;
}) {
  const card = personCard(author);
  return (
    <Floor
      floor={post.floor}
      author={card}
      badges={{
        lg: <PresenceDot userId={author.id} avatarSize={45} />,
        sm: <PresenceDot userId={author.id} avatarSize={32} />,
      }}
      isOp={post.author_id === topic.author_id}
      timeIso={post.created_at}
      time={formatActivity(post.created_at, now)}
      timeFull={formatTime(post.created_at)}
      edited={post.edited_at ? `编辑于 ${formatTime(post.edited_at)}` : undefined}
    >
      <FloorContent
        topicId={topic.id}
        postId={post.id}
        floor={post.floor}
        author={card.name}
        quoteHref={topicHref(topic.id, page, post.floor)}
        title={post.floor === 1 ? topic.title : undefined}
        deleted={post.deleted}
        perms={perms}
        mentionable={mentionable}
        social={post.deleted ? null : social}
        viewer={viewer}
        canModerate={canModerate}
        canReply={canReply}
        mod={mod}
      >
        {post.deleted ? (
          <p className="text-fg-muted italic">该楼层已被删除</p>
        ) : (
          <Prose html={post.html} />
        )}
      </FloorContent>
    </Floor>
  );
}

/** 主题页（或交易帖）这一页的全部楼层：点赞评论、编辑删除权限与管理菜单按访问者算好。 */
export function LiveFloors({
  data,
  me,
  page,
  now,
}: {
  data: Schemas["TopicPage"];
  me: Me | null;
  page: number;
  now: Date;
}) {
  const { topic, forum, posts, users } = data;
  const user = cardLookup(users);
  const people = peopleOf(users);
  const social = new Map(data.social.map((s) => [s.post_id, socialView(s, people)]));
  const mentionable = users.filter((u) => u.id !== me?.id);
  const viewer = me ? { id: me.id, username: me.username ?? null } : null;
  const staff = me?.staff ?? null;
  return posts.map((p) => {
    const own = me?.id === p.author_id;
    const author = user(p.author_id);
    return (
      <LiveFloor
        key={p.id}
        post={p}
        author={author}
        topic={topic}
        page={page}
        mentionable={mentionable}
        social={social.get(p.id) ?? null}
        viewer={viewer}
        canModerate={can(staff, "post_delete", forum.id)}
        canReply={data.can_reply}
        mod={
          data.can_moderate && staff
            ? {
                forumId: forum.id,
                author: { id: author.id, name: userName(author) },
                staff,
                isOwn: own,
              }
            : undefined
        }
        now={now}
        perms={{
          canEdit: !p.deleted && (data.can_moderate || (own && !topic.flags.locked)),
          asModerator: data.can_moderate && !own,
          hasHistory: p.edited_at != null && (!p.deleted || data.can_moderate),
          own,
        }}
      />
    );
  });
}
