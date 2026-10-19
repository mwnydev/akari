import { Amount, Avatar, Badge } from "@akari/ui";
import { Lock, MessageSquare } from "lucide-react";
import type { Topic } from "../lib/mock";

export function TopicRow({ t }: { t: Topic }) {
  const href = t.trade ? `/trade/${t.id}` : `/t/${t.id}`;
  return (
    <li className="flex gap-3 px-4 py-3">
      <Avatar name={t.author} size={36} className="mt-0.5" />
      <div className="min-w-0 flex-1">
        <a href={href} className="line-clamp-2 font-medium hover:text-accent">
          {t.pinned ? (
            <Badge tone="accent" className="mr-1.5 align-[2px]">
              置顶
            </Badge>
          ) : null}
          {t.digest ? (
            <Badge tone="success" className="mr-1.5 align-[2px]">
              精华
            </Badge>
          ) : null}
          {t.trade ? (
            <Badge tone="warning" className="mr-1.5 align-[2px]">
              {t.trade.mode === "stream" ? "分期担保" : "担保"}
            </Badge>
          ) : null}
          {t.title}
          {t.locked ? (
            <Lock size={14} className="ml-1 inline text-fg-muted" aria-label="已锁定" />
          ) : null}
        </a>
        <p className="mt-0.5 flex flex-wrap items-center gap-x-3 text-sm text-fg-muted">
          <span>{t.author}</span>
          <span>
            {t.lastPoster} 回复于 {t.lastAt}
          </span>
          {t.trade ? (
            <span className="font-medium text-fg">
              <Amount value={t.trade.price} />
            </span>
          ) : null}
        </p>
      </div>
      <div className="amount flex shrink-0 items-start gap-1 pt-0.5 text-sm text-fg-muted">
        <MessageSquare size={14} className="mt-1" aria-hidden />
        {t.replies}
      </div>
    </li>
  );
}
