import { Avatar } from "@akari/ui";
import { LogOut, Settings, User } from "lucide-react";
import { LogoutButton } from "./LogoutButton";
import type { HeaderViewer } from "./SiteHeader";

const ID = "account-menu";

const item =
  "flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-fg hover:bg-surface-2 " +
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent disabled:opacity-60";

/**
 * 顶栏头像：点开账号菜单（个人主页、账号设置、退出）。浏览器原生 popover，不需要 JS：
 * Esc、点外面都会收起；菜单里是普通链接，只有「退出」是客户端组件。
 */
export function AccountMenu({ viewer }: { viewer: HeaderViewer }) {
  return (
    <>
      <button
        type="button"
        popoverTarget={ID}
        aria-label={`${viewer.name}：账号菜单`}
        data-testid="nav-user"
        className="ml-1 flex rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
      >
        <Avatar name={viewer.name} src={viewer.avatarUrl ?? undefined} size={32} />
      </button>
      <div
        id={ID}
        popover="auto"
        className="fixed inset-auto top-14 right-2 m-0 w-48 rounded-lg border border-border bg-surface px-0 py-1 text-fg shadow-sheet sm:right-4"
      >
        <p className="truncate px-3 py-1.5 text-xs text-fg-muted">{viewer.name}</p>
        <a href={viewer.profileHref} className={item}>
          <User size={16} aria-hidden />
          个人主页
        </a>
        <a href="/settings" className={item}>
          <Settings size={16} aria-hidden />
          账号设置
        </a>
        <div className="my-1 border-t border-border" />
        <LogoutButton className={item}>
          <LogOut size={16} aria-hidden />
          退出
        </LogoutButton>
      </div>
    </>
  );
}
