import { Button, GlassNav } from "@akari/ui";
import { Bell, Search } from "lucide-react";

const links = [
  { href: "/", label: "首页" },
  { href: "/f/digital", label: "交易" },
  { href: "/f/tech", label: "技术" },
  { href: "/f/help", label: "帮助" },
  { href: "/design", label: "设计系统" },
];

export function SiteNav({ active }: { active?: string }) {
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
          <Button variant="ghost" size="sm" aria-label="通知" className="px-2">
            <Bell size={18} aria-hidden />
            <span className="sr-only sm:not-sr-only">通知</span>
          </Button>
        </>
      }
    />
  );
}
