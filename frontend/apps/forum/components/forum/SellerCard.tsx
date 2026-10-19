import { Card, UserCard } from "@akari/ui";
import type { ReactNode } from "react";
import type { PersonCard } from "./UserPopover";

/**
 * 卖家名片：在线点、等级、信誉（成功率只统计链上已最终确认的订单）。`facts` 是名片下的补充说明
 * （在售件数、加入时间等，有才显示）；`actions` 放「私信卖家」等按钮。
 */
export function SellerCard({
  seller,
  avatarBadge,
  facts,
  actions,
}: {
  seller: PersonCard;
  avatarBadge?: ReactNode;
  facts?: string | undefined;
  actions?: ReactNode;
}) {
  return (
    <Card className="space-y-3">
      <h2 className="text-sm font-medium text-fg-muted">卖家</h2>
      <UserCard
        name={seller.name}
        avatar={seller.avatarUrl ?? undefined}
        level={seller.level}
        levelName={seller.levelName}
        online={seller.online}
        avatarBadge={avatarBadge}
        lastSeen={seller.lastSeen}
        reputation={seller.reputation}
        href={seller.href}
      />
      <p className="text-xs text-fg-muted">
        成功率只统计链上已最终确认的订单；处理中的订单不计入。{facts ? ` ${facts}` : null}
      </p>
      {actions}
    </Card>
  );
}
