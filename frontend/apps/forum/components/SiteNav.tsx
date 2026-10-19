import type { Me } from "@akari/api-client";
import { Avatar, Button, GlassNav } from "@akari/ui";
import { Bell, MessageCircle, Search } from "lucide-react";
import Link from "next/link";
import { profileHref } from "../lib/display";
import { getMe } from "../lib/session";
import { UnreadBadge } from "./live/UnreadBadge";
import { LogoutButton } from "./LogoutButton";

const iconLink =
  "relative inline-flex h-8 items-center gap-1 rounded-pill px-2 text-sm hover:bg-surface-2";

const links = [
  { href: "/", label: "首页" },
  { href: "/f/general", label: "综合" },
  { href: "/f/trade", label: "交易" },
  { href: "/f/tech", label: "技术" },
  { href: "/f/help", label: "帮助" },
  { href: "/design", label: "设计系统" },
];

/** 未设置用户名时显示邮箱 @ 前的部分（只有本人看得到自己的邮箱）。 */
function displayName(me: Me): string {
  return me.username || me.email.split("@")[0] || me.email;
}

export async function SiteNav({ active }: { active?: string }) {
  const me = await getMe();
  return (
    <GlassNav
      brand="Akari"
      links={links.map((l) => ({ ...l, active: l.href === active }))}
      right={
        <>
          <Button variant="ghost" size="sm" aria-label="搜索" className="px-2">
            <Search size={18} aria-hidden />
            <span className="sr-only sm:not-sr-only">搜索</span>
          </Button>
          {me ? (
            <>
              <a href="/messages" className={iconLink}>
                <MessageCircle size={18} aria-hidden />
                <span className="sr-only sm:not-sr-only">私信</span>
              </a>
              <a href="/notifications" className={iconLink}>
                <Bell size={18} aria-hidden />
                <span className="sr-only sm:not-sr-only">通知</span>
                <UnreadBadge />
              </a>
              <Link
                href={profileHref(me)}
                className="flex items-center gap-2"
                data-testid="nav-user"
              >
                <Avatar name={displayName(me)} size={28} />
                <span className="hidden max-w-32 truncate text-sm sm:inline">
                  {displayName(me)}
                </span>
              </Link>
              <LogoutButton />
            </>
          ) : (
            <Button asChild size="sm">
              <Link href="/login">登录</Link>
            </Button>
          )}
        </>
      }
    />
  );
}
