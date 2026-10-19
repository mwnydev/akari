"use client";

import { Button, GlassNav } from "@akari/ui";
import { LogOut } from "lucide-react";
import { useAdminSession } from "./session/SessionProvider";

const links = [
  { href: "/", label: "概览" },
  { href: "/arbitration/", label: "仲裁" },
  { href: "/staff/", label: "人员" },
  { href: "/boards/", label: "板块" },
  { href: "/suspensions/", label: "权限暂停" },
  { href: "/revoke/", label: "批量撤销" },
  { href: "/audit/", label: "审计日志" },
  { href: "/settings/", label: "限速与阈值" },
];

export function AdminNav({ active }: { active: string }) {
  const { logout } = useAdminSession();
  return (
    <GlassNav
      brand="Akari 后台"
      links={links.map((l) => ({ ...l, active: l.href === active }))}
      right={
        <Button variant="ghost" size="sm" onClick={() => void logout()}>
          <LogOut size={16} aria-hidden />
          <span className="sr-only sm:not-sr-only">退出</span>
        </Button>
      }
    />
  );
}
