import { GlassNav, NetworkBadge } from "@akari/ui";

const links = [
  { href: "/", label: "钱包" },
  { href: "/orders/1001", label: "订单" },
  { href: "/checkout", label: "下单" },
];

export function WalletNav({ active }: { active?: string }) {
  return (
    <GlassNav
      brand="Akari 钱包"
      links={links.map((l) => ({ ...l, active: l.href === active }))}
      right={<NetworkBadge className="hidden sm:inline-flex" />}
    />
  );
}
