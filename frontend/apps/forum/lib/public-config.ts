// 浏览器也能读的配置：NEXT_PUBLIC_* 在构建时内联。

/**
 * 钱包中心（独立域名，plan 第 5 节）：担保付款、开通交易都在那里完成。
 * 生产构建默认 ADR 0012 正式域名（只经代码审查修改，不进后台在线配置），`next dev` 默认本机。
 */
export const WALLET_ORIGIN =
  process.env.NEXT_PUBLIC_WALLET_ORIGIN ??
  (process.env.NODE_ENV === "production" ? "https://wallet.akari.cc" : "http://localhost:3001");

/**
 * Cloudflare Turnstile site key；未设置时登录页与新用户发帖都不渲染人机验证（只用于开发：后端没配置
 * TURNSTILE_SECRET 时也不校验，测试与生产的后端缺了密钥拒绝启动）。
 */
export const TURNSTILE_SITE_KEY = process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY ?? "";

/** 发主题、回帖、评论要先过人机验证：站点配置了 Turnstile，且访问者注册不满 24 小时。 */
export function needsPostCaptcha(me: { new_user: boolean } | null | undefined): boolean {
  return TURNSTILE_SITE_KEY !== "" && me?.new_user === true;
}

/** 每页楼层数，与后端 `forum_db::posts::PAGE_SIZE` 一致。 */
export const POSTS_PER_PAGE = 20;
/** 板块前 200 页用页码，之后用游标（后端 `forums::MAX_PAGE`）。 */
export const BOARD_MAX_PAGE = 200;
