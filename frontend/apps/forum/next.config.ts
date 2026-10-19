import type { NextConfig } from "next";

// 浏览器统一请求同源 /api/v1：生产由 Caddy 在到达 Next.js 之前转给 forum-api；
// 开发与本地 next start 时由这里的 rewrite 转发（Origin 头原样保留，后端照常校验）。
const API_ORIGIN = process.env.API_ORIGIN ?? "http://127.0.0.1:8080";

const config: NextConfig = {
  output: "standalone",
  reactStrictMode: true,
  poweredByHeader: false,
  // 图标按需打包（plan 第 12 节：首次加载 JS < 200 KB）；组件库靠子路径拆分，见 packages/ui/src/index.ts
  experimental: { optimizePackageImports: ["lucide-react"] },
  transpilePackages: ["@akari/ui", "@akari/fees", "@akari/api-client"],
  async rewrites() {
    return [{ source: "/api/v1/:path*", destination: `${API_ORIGIN}/api/v1/:path*` }];
  },
};

export default config;
