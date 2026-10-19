import { Card, UserCard } from "@akari/ui";
import type { Floor as FloorData } from "../lib/mock";
import { user } from "../lib/mock";
import { Prose } from "./Prose";

export function Floor({ f }: { f: FloorData }) {
  const u = user(f.author);
  return (
    <Card id={`floor-${f.floor}`} className="scroll-mt-20">
      <div className="flex items-start justify-between gap-3">
        <UserCard
          name={u.name}
          level={u.level}
          levelName={u.levelName}
          online={u.online}
          lastSeen={u.lastSeen}
          reputation={u.reputation}
          href={`/u/${encodeURIComponent(u.name)}`}
          size="sm"
        />
        <a href={`#floor-${f.floor}`} className="amount shrink-0 text-sm text-fg-muted">
          #{f.floor}
        </a>
      </div>
      <div className="mt-3">
        {f.deleted ? (
          <p className="text-fg-muted italic">该楼层已被删除</p>
        ) : (
          <>
            {f.quote ? (
              <blockquote className="mb-3 rounded-md border-l-4 border-accent bg-surface-2 px-3 py-2 text-sm">
                <a href={`#floor-${f.quote.floor}`} className="text-fg-muted">
                  引用 #{f.quote.floor} {f.quote.author}：
                </a>
                {f.quote.text}
              </blockquote>
            ) : null}
            <Prose html={f.html} />
          </>
        )}
      </div>
      <p className="mt-3 text-xs text-fg-muted">
        {f.at}
        {f.edited ? " · 已编辑（查看历史）" : ""}
      </p>
    </Card>
  );
}
