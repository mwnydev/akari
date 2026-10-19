import { Button, Card, UserCard } from "@akari/ui";
import { MessageCircle } from "lucide-react";
import type { MockUser } from "../_mock/data";

/** 卖家名片：在线点、等级、信誉（成功率只统计链上已最终确认的订单）、回复用时。 */
export function SellerCard({ seller, onSale }: { seller: MockUser; onSale: number }) {
  return (
    <Card className="space-y-3">
      <h2 className="text-sm font-medium text-fg-muted">卖家</h2>
      <UserCard
        name={seller.name}
        level={seller.level}
        levelName={seller.levelName}
        online={seller.online}
        lastSeen={seller.lastSeen}
        reputation={seller.reputation}
        href="#"
      />
      <p className="text-xs text-fg-muted">
        成功率只统计链上已最终确认的订单；处理中的订单不计入。在售 {onSale} 件 · 加入于{" "}
        {seller.joined}
      </p>
      <Button variant="secondary" size="sm" className="w-full">
        <MessageCircle size={16} aria-hidden />
        私信卖家
      </Button>
    </Card>
  );
}
