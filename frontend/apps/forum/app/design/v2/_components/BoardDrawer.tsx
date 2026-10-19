import { LazyDrawer } from "@akari/ui/lazy";
import { Menu } from "lucide-react";
import { BoardNav } from "./BoardNav";
import { headerIcon } from "./styles";

/** 窄屏顶栏左侧的汉堡按钮：侧栏收进左侧抽屉（抽屉代码点开时才加载）。 */
export function BoardDrawer({ active }: { active: string }) {
  return (
    <LazyDrawer
      title="导航"
      trigger={
        <button type="button" aria-label="打开导航" className={headerIcon}>
          <Menu size={20} aria-hidden />
        </button>
      }
    >
      <BoardNav active={active} />
    </LazyDrawer>
  );
}
