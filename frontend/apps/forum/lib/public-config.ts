// 浏览器也能读的配置：NEXT_PUBLIC_* 在构建时内联。

/** 钱包中心（独立域名，plan 第 5 节）：担保付款、开通交易都在那里完成。 */
export const WALLET_ORIGIN = process.env.NEXT_PUBLIC_WALLET_ORIGIN ?? "http://localhost:3001";

/** Cloudflare Turnstile site key；未设置时登录页不渲染人机验证（后端未配置 TURNSTILE_SECRET 时也不校验）。 */
export const TURNSTILE_SITE_KEY = process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY ?? "";

/** 每页楼层数，与后端 `forum_db::posts::PAGE_SIZE` 一致。 */
export const POSTS_PER_PAGE = 20;
/** 板块前 200 页用页码，之后用游标（后端 `forums::MAX_PAGE`）。 */
export const BOARD_MAX_PAGE = 200;
