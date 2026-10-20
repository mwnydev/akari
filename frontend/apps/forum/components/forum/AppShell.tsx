import type { ReactNode } from "react";
import { headerViewer } from "../../lib/nav";
import { getMe } from "../../lib/session";
import { canHandleReports, isArbitrator } from "../../lib/staff";
import { UnreadBadge } from "../live/UnreadBadge";
import { UsernameGate } from "../settings/UsernameGate";
import { AccountMenu } from "./AccountMenu";
import { ForumShell } from "./ForumShell";
import { shellNav } from "./shell-nav";
import { SiteHeader } from "./SiteHeader";

/**
 * 正式页面的骨架：顶栏（登录状态）+ 侧栏（接口里访问者看得到的板块）+ 正文。
 * 游客渲染时不带会话，侧栏与顶栏都与访问者无关，整页仍可缓存；未读数在浏览器里取。
 * `active` 是侧栏高亮项：home / hot 或板块 slug。
 * 登录了还没设置用户名的，外面包一层 UsernameGate（第一次弹出设置用户名，交易入口再问）。
 */
export async function AppShell({
  active = "",
  className,
  children,
}: {
  active?: string;
  className?: string | undefined;
  children: ReactNode;
}) {
  const [me, nav] = await Promise.all([getMe(), shellNav(active)]);
  const viewer = me ? headerViewer(me) : null;
  return (
    <UsernameGate userId={me && !me.username ? me.id : null}>
      <ForumShell
        header={
          <SiteHeader
            nav={nav}
            viewer={viewer}
            unread={<UnreadBadge />}
            account={
              viewer ? (
                <AccountMenu viewer={viewer} reports={canHandleReports(me?.staff)} />
              ) : undefined
            }
            arbitration={isArbitrator(me?.staff)}
          />
        }
        nav={nav}
        className={className}
      >
        {children}
      </ForumShell>
    </UsernameGate>
  );
}
