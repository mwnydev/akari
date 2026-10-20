// 游客页面的边缘缓存头（proxy.ts 设置，端到端测试核对）。不引 next/server，测试可以直接导入。

const GUEST_EDGE_TTL = 60;
// 过期后边缘节点先返回旧页面、同时后台回源（只多一次回源的时间，不会多出 30 秒的旧内容）；源站出错时最多再用 5 分钟旧页面。
const GUEST_EDGE_STALE = 30;
const GUEST_EDGE_IF_ERROR = 300;
export const GUEST_EDGE_CACHE = `public, max-age=${GUEST_EDGE_TTL}, stale-while-revalidate=${GUEST_EDGE_STALE}, stale-if-error=${GUEST_EDGE_IF_ERROR}`;
