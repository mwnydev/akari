import type { Schemas } from "@akari/api-client";
import { Avatar } from "@akari/ui";
import { MessageSquare } from "lucide-react";
import Link from "next/link";
import { formatTime, userName, type UserCardView } from "../lib/display";
import { TopicBadges } from "./TopicBadges";

export function TopicRow({
  topic,
  user,
}: {
  topic: Schemas["TopicSummary"];
  user: (id: number) => UserCardView;
}) {
  const author = userName(user(topic.author_id));
  const lastPoster = topic.last_poster_id ? userName(user(topic.last_poster_id)) : author;
  return (
    <li className="flex gap-3 px-4 py-3">
      <Avatar name={author} size={36} className="mt-0.5" />
      <div className="min-w-0 flex-1">
        <Link href={`/t/${topic.id}`} className="line-clamp-2 font-medium hover:text-accent">
          <TopicBadges topic={topic} />
          {topic.title}
        </Link>
        <p className="mt-0.5 flex flex-wrap items-center gap-x-3 text-sm text-fg-muted">
          <span>{author}</span>
          <span>
            {topic.replies > 0 ? `${lastPoster} 回复于 ` : "发布于 "}
            <time dateTime={topic.last_post_at}>{formatTime(topic.last_post_at)}</time>
          </span>
        </p>
      </div>
      <div
        className="amount flex shrink-0 items-start gap-1 pt-0.5 text-sm text-fg-muted"
        aria-label={`${topic.replies} 条回复`}
      >
        <MessageSquare size={14} className="mt-1" aria-hidden />
        {topic.replies}
      </div>
    </li>
  );
}
