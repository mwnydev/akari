import Link from "next/link";
import type { BoardColor } from "../../lib/board-color";
import { BoardSquare } from "./BoardMark";

export interface DirectoryBoard {
  slug: string;
  name: string;
  color: BoardColor;
  href: string;
  description: string;
  /** 主题数、今日新帖：接口有这项数据时才显示对应的列 */
  topics?: number | undefined;
  today?: number | undefined;
  children?: { slug: string; name: string; href: string }[] | undefined;
}

const th = "w-20 px-2 py-2 text-right font-normal";

/** 「板块」标签页：全部板块一览，表格行，左侧色块标出板块，子板块列在说明下面。 */
export function BoardDirectory({ boards }: { boards: DirectoryBoard[] }) {
  const counts = boards.some((b) => b.topics !== undefined);
  return (
    <table className="w-full border-collapse">
      <caption className="sr-only">全部板块</caption>
      <thead>
        <tr className="border-b border-border text-left text-sm text-fg-muted">
          <th scope="col" className="px-2 py-2 font-normal">
            板块
          </th>
          {counts ? (
            <>
              <th scope="col" className={th}>
                主题
              </th>
              <th scope="col" className={th}>
                今日
              </th>
            </>
          ) : null}
        </tr>
      </thead>
      <tbody>
        {boards.map((b) => (
          <tr key={b.slug} className="border-b border-border">
            <td className="px-2 py-3">
              <Link
                href={b.href}
                className="inline-flex items-center gap-2 rounded-sm text-[17px] focus-visible:outline-2 focus-visible:outline-accent"
              >
                <BoardSquare color={b.color} className="size-3" />
                {b.name}
              </Link>
              <p className="mt-0.5 text-sm text-fg-muted">{b.description}</p>
              {b.children?.length ? (
                <ul className="mt-1 flex flex-wrap gap-x-3 text-sm">
                  {b.children.map((c) => (
                    <li key={c.slug}>
                      <Link href={c.href} className="text-accent hover:underline">
                        {c.name}
                      </Link>
                    </li>
                  ))}
                </ul>
              ) : null}
            </td>
            {counts ? (
              <>
                <td className="amount px-2 text-right text-fg-muted">{b.topics ?? "—"}</td>
                <td className="amount px-2 text-right text-fg-muted">{b.today ?? "—"}</td>
              </>
            ) : null}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
