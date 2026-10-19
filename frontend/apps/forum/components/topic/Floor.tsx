import type { Schemas } from "@akari/api-client";
import { Card, UserCard } from "@akari/ui";
import type { ReactNode } from "react";
import { formatTime, profileHref, userName, type UserCardView } from "../../lib/display";
import { PresenceDot } from "../live/PresenceDot";
import { Prose } from "../Prose";
import { FloorShell, type FloorPermissions } from "./FloorShell";

export function Floor({
  post,
  author,
  title,
  perms,
  mentionable,
  mod,
}: {
  post: Schemas["PostView"];
  author: UserCardView;
  /** 1 楼传主题标题（编辑时可一并修改） */
  title?: string | undefined;
  perms: FloorPermissions;
  mentionable: UserCardView[];
  /** 管理菜单（管理人员才有） */
  mod?: ReactNode;
}) {
  const meta = (
    <>
      <time dateTime={post.created_at}>{formatTime(post.created_at)}</time>
      {post.edited_at ? ` · 编辑于 ${formatTime(post.edited_at)}` : ""}
    </>
  );
  return (
    <Card id={`floor-${post.floor}`} className="scroll-mt-20" data-testid="floor">
      <div className="flex items-start justify-between gap-3">
        <UserCard
          name={userName(author)}
          level={author.level}
          href={profileHref(author)}
          avatar={author.avatar_url ?? undefined}
          avatarBadge={<PresenceDot userId={author.id} avatarSize={32} />}
          size="sm"
        />
        <span className="flex shrink-0 items-center gap-1">
          {mod}
          <a href={`#floor-${post.floor}`} className="amount text-sm text-fg-muted">
            #{post.floor}
          </a>
        </span>
      </div>
      <FloorShell
        postId={post.id}
        floor={post.floor}
        title={title}
        perms={perms}
        mentionable={mentionable}
        meta={meta}
      >
        {post.deleted ? (
          <p className="text-fg-muted italic">该楼层已被删除</p>
        ) : (
          <Prose html={post.html} />
        )}
      </FloorShell>
    </Card>
  );
}
