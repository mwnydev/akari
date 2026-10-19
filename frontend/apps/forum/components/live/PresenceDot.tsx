"use client";

import { OnlineDot } from "@akari/ui";
import { useEffect, useState } from "react";
import { watchPresence, type Presence } from "../../lib/presence";

/** 某人的在线状态（浏览器里查询，游客缓存的页面里也不含按访问者变化的数据）；还不知道时为 null。 */
export function usePresence(userId: number | null): Presence | null {
  const [p, setP] = useState<Presence | null>(null);
  useEffect(() => {
    if (userId === null) return;
    return watchPresence(userId, setP);
  }, [userId]);
  return p && p.user_id === userId ? p : null;
}

/** 叠在头像右下角的在线点（传给 Avatar / UserCard 的 badge）。 */
export function PresenceDot({ userId, avatarSize }: { userId: number; avatarSize: number }) {
  const p = usePresence(userId);
  return p ? <OnlineDot online={p.online} avatarSize={avatarSize} /> : null;
}
