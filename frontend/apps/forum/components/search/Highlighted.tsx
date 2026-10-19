import type { Schemas } from "@akari/api-client";

type Highlight = Schemas["Highlight"];

/**
 * 搜索结果的高亮：后端已经切好「文字 / 命中」片段，这里全部当纯文本渲染（React 会转义），不经过 HTML。
 * `before` / `after` 为真时在前后加省略号（片段被截断）。片段没有别的稳定标识、顺序也不变，用下标作 key。
 */
export function Highlighted({
  parts,
  before = false,
  after = false,
}: {
  parts: Highlight[];
  before?: boolean;
  after?: boolean;
}) {
  return (
    <>
      {before ? "…" : null}
      {parts.map((p, i) =>
        p.hit ? (
          <mark key={i} className="rounded-sm bg-accent-soft px-0.5 text-fg">
            {p.text}
          </mark>
        ) : (
          <span key={i}>{p.text}</span>
        ),
      )}
      {after ? "…" : null}
    </>
  );
}
