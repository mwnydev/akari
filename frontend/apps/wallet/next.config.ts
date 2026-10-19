import type { NextConfig } from "next";
import { PHASE_DEVELOPMENT_SERVER, PHASE_PRODUCTION_BUILD } from "next/constants";
import { API_BASE, FORUM_ORIGIN, RP_ID, domainProblems } from "./lib/wallet/domains";

/** 本地 forum-api；只给 `next dev` 的同源转发用（静态导出没有服务端，生产直连 NEXT_PUBLIC_API_ORIGIN）。 */
const API_ORIGIN = process.env.API_ORIGIN ?? "http://127.0.0.1:8080";

export default function nextConfig(phase: string): NextConfig {
  const dev = phase === PHASE_DEVELOPMENT_SERVER;
  // DEV 代付（lib/wallet/dev-sponsor.ts）内含开发私钥，只允许 `next dev`；任何构建产物都不能带上它。
  // 光靠 `if (DEV_SPONSOR)` 不够：Turbopack 仍会为死分支里的动态 import 生成分包，所以在解析层就换成空实现，
  // 构建后 scripts/assert-no-dev-sponsor.mjs 再扫一遍产物。
  const devSponsor = process.env.NEXT_PUBLIC_DEV_SPONSOR === "1";
  if (devSponsor && phase === PHASE_PRODUCTION_BUILD) {
    throw new Error("NEXT_PUBLIC_DEV_SPONSOR=1 只能用于 next dev，禁止出现在构建产物里");
  }
  if (phase === PHASE_PRODUCTION_BUILD) {
    const problems = domainProblems({ rpId: RP_ID, apiBase: API_BASE, forumOrigin: FORUM_ORIGIN });
    if (problems.length > 0)
      throw new Error(`钱包域名配置不合格（ADR 0012）：\n${problems.join("\n")}`);
  }
  const devSponsorModule =
    devSponsor && dev ? "./lib/wallet/dev-sponsor.ts" : "./lib/wallet/dev-sponsor.disabled.ts";

  return {
    output: "export",
    reactStrictMode: true,
    poweredByHeader: false,
    // 图标按需打包（plan 第 12 节：首次加载 JS < 200 KB）；组件库靠子路径拆分，见 packages/ui/src/index.ts
    experimental: { optimizePackageImports: ["lucide-react"] },
    transpilePackages: ["@akari/ui", "@akari/fees", "@akari/contracts", "@akari/api-client"],
    turbopack: { resolveAlias: { "akari-dev-sponsor": devSponsorModule } },
    // 开发时钱包页面同源请求 /api/v1，由这里转给 forum-api（Origin 原样保留，后端照常校验 WALLET_ORIGINS）
    ...(dev && {
      rewrites: async () => [
        { source: "/api/v1/:path*", destination: `${API_ORIGIN}/api/v1/:path*` },
      ],
    }),
  };
}
