import { boards } from "../_mock/data";
import { BoardSquare } from "./BoardMark";

/** 「板块」标签页：全部板块一览，表格行，左侧色块标出板块。 */
export function BoardDirectory() {
  return (
    <table className="w-full border-collapse">
      <caption className="sr-only">全部板块</caption>
      <thead>
        <tr className="border-b border-border text-left text-sm text-fg-muted">
          <th scope="col" className="px-2 py-2 font-normal">
            板块
          </th>
          <th scope="col" className="w-20 px-2 py-2 text-right font-normal">
            主题
          </th>
          <th scope="col" className="w-20 px-2 py-2 text-right font-normal">
            今日
          </th>
        </tr>
      </thead>
      <tbody>
        {boards.map((b) => (
          <tr key={b.slug} className="border-b border-border">
            <td className="px-2 py-3">
              <a
                href={b.slug === "trade" ? "/design/v2/board" : "#"}
                className="inline-flex items-center gap-2 rounded-sm text-[17px] focus-visible:outline-2 focus-visible:outline-accent"
              >
                <BoardSquare color={b.color} className="size-3" />
                {b.name}
              </a>
              <p className="mt-0.5 text-sm text-fg-muted">{b.description}</p>
            </td>
            <td className="amount px-2 text-right text-fg-muted">{b.topics}</td>
            <td className="amount px-2 text-right text-fg-muted">{b.today}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
