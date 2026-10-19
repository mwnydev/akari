import type { NextConfig } from "next";

const config: NextConfig = {
  output: "export",
  // 每页导出成 目录/index.html：任何静态托管（Cloudflare Pages、本地测试用的简单静态服务）都能按路径直接访问
  trailingSlash: true,
  reactStrictMode: true,
  poweredByHeader: false,
  // 图标按需打包（plan 第 12 节：首次加载 JS < 200 KB）；组件库靠子路径拆分，见 packages/ui/src/index.ts
  experimental: { optimizePackageImports: ["lucide-react"] },
  transpilePackages: ["@akari/ui", "@akari/api-client"],
};

export default config;
