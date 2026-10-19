import { Avatar } from "@akari/ui";
import { Bell, Mail, Search } from "lucide-react";
import { BoardDrawer } from "./BoardDrawer";
import { headerIcon } from "./styles";

/** 设计稿里的未读通知数。 */
const UNREAD_NOTIFICATIONS = 3;

/**
 * 顶栏：通栏实底、底部 1px 分隔线。左边站名（窄屏前面多一个打开侧栏的汉堡按钮），右边搜索、私信、通知与头像。
 * 设计稿不读会话，固定显示「已登录」状态。
 */
export function SiteHeader({ active }: { active: string }) {
  return (
    <header className="sticky top-0 z-30 h-14 border-b border-border bg-surface">
      <div className="mx-auto flex h-full max-w-[90rem] items-center gap-1 px-2 sm:px-4">
        <div className="lg:hidden">
          <BoardDrawer active={active} />
        </div>
        <a
          href="/design/v2"
          className="ml-1 flex items-center gap-2 rounded-sm text-lg font-semibold focus-visible:outline-2 focus-visible:outline-accent"
        >
          <span
            aria-hidden
            className="flex size-7 items-center justify-center rounded-md bg-accent-fill text-sm font-bold text-accent-fg"
          >
            A
          </span>
          Akari
        </a>
        <div className="ml-auto flex items-center gap-0.5">
          <button type="button" aria-label="搜索" className={headerIcon}>
            <Search size={20} aria-hidden />
          </button>
          <a href="#" aria-label="私信" className={headerIcon}>
            <Mail size={20} aria-hidden />
          </a>
          <a href="#" aria-label={`通知，${UNREAD_NOTIFICATIONS} 条未读`} className={headerIcon}>
            <Bell size={20} aria-hidden />
            <span
              aria-hidden
              className="amount absolute top-1 right-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-accent-fill px-1 text-[11px] leading-none font-medium text-accent-fg"
            >
              {UNREAD_NOTIFICATIONS}
            </span>
          </a>
          <a
            href="#"
            aria-label="小林的个人主页"
            className="ml-1 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          >
            <Avatar name="小林" size={32} />
          </a>
        </div>
      </div>
    </header>
  );
}
