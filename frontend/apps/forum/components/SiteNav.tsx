import { Avatar, Button, GlassNav } from "@akari/ui";
import { Bell, Search } from "lucide-react";
import Link from "next/link";
import { getMe } from "../lib/session";
import { LogoutButton } from "./LogoutButton";

const links = [
  { href: "/", label: "首页" },
  { href: "/f/digital", label: "交易" },
  { href: "/f/tech", label: "技术" },
  { href: "/f/help", label: "帮助" },
  { href: "/design", label: "设计系统" },
];

/** 未设置用户名时显示邮箱 @ 前的部分。 */
function displayName(me: { username?: string | null; email: string }): string {
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
              <Button variant="ghost" size="sm" aria-label="通知" className="px-2">
                <Bell size={18} aria-hidden />
                <span className="sr-only sm:not-sr-only">通知</span>
              </Button>
              <span className="flex items-center gap-2" data-testid="nav-user">
                <Avatar name={displayName(me)} size={28} />
                <span className="hidden max-w-32 truncate text-sm sm:inline">
                  {displayName(me)}
                </span>
              </span>
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
