import { ApiError } from "@akari/api-client";
import { forumTree, listForums } from "../../lib/forums";
import { navBoards, SHORTCUTS } from "../../lib/nav";
import { BoardNav } from "./BoardNav";

/** 侧栏：接口里访问者看得到的板块（同一次渲染内只请求一次）；接口出错时只剩「浏览」分组。 */
export async function shellNav(active: string) {
  const forums = await listForums();
  const boards = forums instanceof ApiError ? [] : navBoards(forumTree(forums));
  return <BoardNav shortcuts={SHORTCUTS} boards={boards} active={active} />;
}
