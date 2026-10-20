import { GlassNav, NetworkBadge } from "@akari/ui";
import { FORUM_ORIGIN, NETWORK_LABEL } from "../lib/wallet/config";

// 「报价」（/offer）与「付款」（/pay）要从交易帖或私信带着参数进入，直接打开只有一条错误提示，
// 不放进导航；最后一项回论坛（钱包页面原来没有回去的入口）
const links = [
  { href: "/", label: "钱包" },
  { href: "/order", label: "订单" },
  { href: "/setup", label: "开通" },
  { href: FORUM_ORIGIN, label: "回论坛" },
];

export function WalletNav({ active }: { active?: string }) {
  return (
    <GlassNav
      brand="Akari 钱包"
      links={links.map((l) => ({ ...l, active: l.href === active }))}
      // 外包一层再隐藏：徽章自带 inline-flex，直接加 hidden 会被它盖掉（窄屏把导航挤出屏幕）；
      // 网络名按构建的链显示（测试网不能写成 Arbitrum One）
      right={
        <span className="hidden sm:block">
          <NetworkBadge label={NETWORK_LABEL} />
        </span>
      }
    />
  );
}
