import { Menu, X } from "lucide-react";
import type { ReactNode } from "react";
import { headerIcon } from "./styles";

const ID = "nav-drawer";

/**
 * 窄屏顶栏左侧的汉堡按钮：侧栏收进左侧抽屉。用浏览器原生的 popover（不需要 JS）：
 * Esc、点抽屉外面都会关上；抽屉里是普通链接，点了整页跳转。
 */
export function BoardDrawer({ children }: { children: ReactNode }) {
  return (
    <>
      <button type="button" popoverTarget={ID} aria-label="打开导航" className={headerIcon}>
        <Menu size={20} aria-hidden />
      </button>
      <div
        id={ID}
        popover="auto"
        aria-label="导航"
        className="fixed inset-y-0 left-0 m-0 h-dvh max-h-none w-72 max-w-[85vw] overflow-y-auto border-r border-border bg-surface p-3 text-fg shadow-sheet backdrop:bg-black/40"
      >
        <div className="mb-3 flex items-center justify-between pl-2.5">
          <span className="font-semibold">导航</span>
          <button
            type="button"
            popoverTarget={ID}
            popoverTargetAction="hide"
            aria-label="关闭导航"
            className={headerIcon}
          >
            <X size={20} aria-hidden />
          </button>
        </div>
        {children}
      </div>
    </>
  );
}
