/** 重连退避：指数增长、有上限，再乘一个随机因子（等量抖动），避免大量页面在服务重启后同一时刻重连。 */
export interface Backoff {
  /** 第一次重连的基准等待（毫秒） */
  baseMs: number;
  /** 等待上限（毫秒） */
  maxMs: number;
}

export const RECONNECT_BACKOFF: Backoff = { baseMs: 1_000, maxMs: 60_000 };

/**
 * 第 `attempt` 次（从 0 起）重连前等多久：取 `min(max, base·2^attempt)` 的一半，再加上 [0, 另一半) 的随机值。
 * 所以等待总落在 [d/2, d) 之间，既有抖动又不会接近 0。
 */
export function backoffDelay(attempt: number, b: Backoff, random: () => number = Math.random) {
  const exp = Math.min(b.maxMs, b.baseMs * 2 ** Math.min(attempt, 30));
  const half = exp / 2;
  return Math.floor(half + random() * half);
}
