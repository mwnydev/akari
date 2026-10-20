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
import { floorPage } from "../../lib/pagination";
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
  replyTo,
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
  /** 这一楼回复的楼层的作者（`post.reply_to` 不为空时） */
  replyTo: UserCardView | null;
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
  // 隐藏的楼层：管理人员与作者看得到内容（带说明），其他人拿到的内容是空的
  const hiddenFor = post.hidden ? (mod ? "staff" : perms.own ? "author" : "others") : null;
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
      replyTo={
        post.reply_to && replyTo
          ? {
              floor: post.reply_to.floor,
              name: userName(replyTo),
              href: topicHref(topic.id, floorPage(post.reply_to.floor), post.reply_to.floor),
            }
          : undefined
      }
    >
      <FloorContent
        topicId={topic.id}
        postId={post.id}
        floor={post.floor}
        author={card.name}
        quoteHref={topicHref(topic.id, page, post.floor)}
        title={post.floor === 1 ? topic.title : undefined}
        deleted={post.deleted}
        hidden={post.hidden}
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
        ) : hiddenFor === "others" ? (
          <p className="text-fg-muted italic">该楼层已被管理人员隐藏</p>
        ) : (
          <>
            {hiddenFor ? (
              <p
                role="note"
                data-testid="hidden-notice"
                className="mb-2 rounded-md bg-warning-soft px-3 py-1.5 text-sm text-warning"
              >
                {hiddenFor === "staff"
                  ? "已隐藏，只有管理人员和作者看得到"
                  : "这层楼已被管理人员隐藏，其他人看不到"}
                {post.hidden_reason ? `。理由：${post.hidden_reason}` : null}
              </p>
            ) : null}
            <Prose html={post.html} />
          </>
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
        replyTo={p.reply_to ? user(p.reply_to.author_id) : null}
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
          hasHistory:
            p.edited_at != null && (data.can_moderate || (!p.deleted && (!p.hidden || own))),
          own,
        }}
      />
    );
  });
}
