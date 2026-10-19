// 未读角标。单独成文件：导航栏的首屏代码只需要它。

/** 未读数的上限（后端会话未读与通知未读都最多数到 100）。 */
export const UNREAD_CAP = 100;

/** 未读角标文字：超过 99 显示 99+，0 不显示。 */
export function badgeText(count: number): string | null {
  if (count <= 0) return null;
  return count > 99 ? "99+" : String(count);
}
