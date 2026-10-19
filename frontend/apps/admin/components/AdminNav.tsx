import { Badge, GlassNav } from "@akari/ui";

const links = [
  { href: "/", label: "概览" },
  { href: "/disputes", label: "争议" },
  { href: "/", label: "用户" },
  { href: "/", label: "内容" },
  { href: "/", label: "财务" },
];

export function AdminNav({ active }: { active?: string }) {
  return (
    <GlassNav
      brand="Akari 后台"
      links={links.map((l, i) => ({
        ...l,
        href: i < 2 ? l.href : `${l.href}#${l.label}`,
        active: l.href === active && i < 2,
      }))}
      right={<Badge tone="accent">仲裁员 · 张三</Badge>}
    />
  );
}
