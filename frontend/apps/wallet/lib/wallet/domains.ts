// 钱包用到的域名（ADR 0012）。生产构建默认就是正式域名，只经代码审查修改，不进后台在线配置；
// `next dev` 默认本机。构建时 next.config.ts 用 domainProblems 校验，不合格直接构建失败。
// 本文件不能有 import：next.config.ts 也引用它。

const PRODUCTION = process.env.NODE_ENV === "production";

/** 钱包 Passkey 的 RP ID = 钱包中心域名，绝不能是论坛主站 `akari.cc`（主站能请求 RP ID 为它本身的 Passkey）。 */
export const RP_ID =
  process.env.NEXT_PUBLIC_WALLET_RP_ID ?? (PRODUCTION ? "wallet.akari.cc" : "localhost");

/** forum-api 所在源；开发时留空 = 同源 `/api/v1`，由 next.config.ts 的 rewrite 转发到本地 forum-api。 */
export const API_BASE =
  process.env.NEXT_PUBLIC_API_ORIGIN ?? (PRODUCTION ? "https://akari.cc" : "");

/** 论坛站点：钱包本身不做登录，未登录时引导到论坛登录页。 */
export const FORUM_ORIGIN =
  process.env.NEXT_PUBLIC_FORUM_ORIGIN ??
  (PRODUCTION ? "https://akari.cc" : "http://localhost:3000");

/** 论坛主站所在的可注册域名。 */
const FORUM_REGISTRABLE_DOMAIN = "akari.cc";

function within(host: string, domain: string): boolean {
  return host === domain || host.endsWith(`.${domain}`);
}

/** 生产构建的域名检查：来源必须是不带路径的 https 源，RP ID 不能是论坛主站或其父域名。返回所有问题。 */
export function domainProblems(d: {
  rpId: string;
  apiBase: string;
  forumOrigin: string;
}): string[] {
  const problems: string[] = [];
  const forumHosts = [FORUM_REGISTRABLE_DOMAIN];
  for (const [name, value] of [
    ["NEXT_PUBLIC_API_ORIGIN", d.apiBase],
    ["NEXT_PUBLIC_FORUM_ORIGIN", d.forumOrigin],
  ] as const) {
    let url: URL | null = null;
    try {
      url = new URL(value);
    } catch {
      // 下面统一报错
    }
    if (!url || url.protocol !== "https:" || url.origin !== value) {
      problems.push(`${name} must be an https origin without a path, got "${value}"`);
    } else {
      forumHosts.push(url.hostname);
    }
  }
  const host = forumHosts.find((h) => within(h, d.rpId));
  if (host) {
    problems.push(
      `NEXT_PUBLIC_WALLET_RP_ID "${d.rpId}" must not be the forum host ${host} or its parent domain`,
    );
  }
  if (d.rpId === "localhost") {
    problems.push(`NEXT_PUBLIC_WALLET_RP_ID must not be localhost in a production build`);
  }
  return problems;
}
