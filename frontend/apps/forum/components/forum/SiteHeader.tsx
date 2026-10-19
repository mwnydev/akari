import { Avatar, Button } from "@akari/ui";
import { Bell, Gavel, Mail, Search } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { BoardDrawer } from "./BoardDrawer";
import { Brand } from "./Brand";
import { headerIcon } from "./styles";

export interface HeaderViewer {
  name: string;
  avatarUrl?: string | null | undefined;
  profileHref: string;
}

/**
 * 顶栏：通栏实底、底部 1px 分隔线。左边站名（窄屏前面多一个打开侧栏的汉堡按钮），右边搜索、私信、通知与头像。
 * `nav` 是窄屏抽屉里的侧栏；`unread` 叠在通知图标上（自带读屏文字）；`account` 替换默认的头像链接（账号菜单）。
 * `arbitration` 给站长、管理员多一个「仲裁」入口（案件列表）。
 */
export function SiteHeader({
  homeHref = "/",
  nav,
  viewer,
  unread,
  account,
  arbitration = false,
  loginHref = "/login",
}: {
  homeHref?: string;
  nav: ReactNode;
  viewer: HeaderViewer | null;
  unread?: ReactNode;
  account?: ReactNode;
  arbitration?: boolean;
  loginHref?: string;
}) {
  return (
    <header className="sticky top-0 z-30 h-14 border-b border-border bg-surface">
      <div className="mx-auto flex h-full max-w-[90rem] items-center gap-1 px-2 sm:px-4">
        <div className="lg:hidden">
          <BoardDrawer>{nav}</BoardDrawer>
        </div>
        <Brand href={homeHref} />
        <div className="ml-auto flex items-center gap-0.5">
          <button type="button" aria-label="搜索" className={headerIcon}>
            <Search size={20} aria-hidden />
          </button>
          {viewer ? (
            <>
              <a href="/messages" aria-label="私信" className={headerIcon}>
                <Mail size={20} aria-hidden />
              </a>
              {arbitration ? (
                <a href="/arbitration" aria-label="仲裁" className={headerIcon}>
                  <Gavel size={20} aria-hidden />
                </a>
              ) : null}
              <a href="/notifications" className={headerIcon}>
                <Bell size={20} aria-hidden />
                <span className="sr-only">通知</span>
                {unread}
              </a>
              {account ?? (
                <a
                  href={viewer.profileHref}
                  aria-label={`${viewer.name}的个人主页`}
                  className="ml-1 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                  <Avatar name={viewer.name} src={viewer.avatarUrl ?? undefined} size={32} />
                </a>
              )}
            </>
          ) : (
            <Button asChild size="sm" className="ml-1">
              <Link href={loginHref}>登录</Link>
            </Button>
          )}
        </div>
      </div>
    </header>
  );
}
