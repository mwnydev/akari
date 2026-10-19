import type { NextConfig } from "next";
import { PHASE_DEVELOPMENT_SERVER, PHASE_PRODUCTION_BUILD } from "next/constants";

export default function nextConfig(phase: string): NextConfig {
  // DEV 代付（lib/wallet/dev-sponsor.ts）内含开发私钥，只允许 `next dev`；任何构建产物都不能带上它。
  // 光靠 `if (DEV_SPONSOR)` 不够：Turbopack 仍会为死分支里的动态 import 生成分包，所以在解析层就换成空实现，
  // 构建后 scripts/assert-no-dev-sponsor.mjs 再扫一遍产物。
  const devSponsor = process.env.NEXT_PUBLIC_DEV_SPONSOR === "1";
  if (devSponsor && phase === PHASE_PRODUCTION_BUILD) {
    throw new Error("NEXT_PUBLIC_DEV_SPONSOR=1 只能用于 next dev，禁止出现在构建产物里");
  }
  const devSponsorModule =
    devSponsor && phase === PHASE_DEVELOPMENT_SERVER
      ? "./lib/wallet/dev-sponsor.ts"
      : "./lib/wallet/dev-sponsor.disabled.ts";

  return {
    output: "export",
    reactStrictMode: true,
    poweredByHeader: false,
    // 图标按需打包（plan 第 12 节：首次加载 JS < 200 KB）；组件库靠子路径拆分，见 packages/ui/src/index.ts
    experimental: { optimizePackageImports: ["lucide-react"] },
    transpilePackages: ["@akari/ui", "@akari/fees", "@akari/contracts"],
    turbopack: { resolveAlias: { "akari-dev-sponsor": devSponsorModule } },
  };
}
