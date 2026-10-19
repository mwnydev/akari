import type { NextConfig } from "next";

const config: NextConfig = {
  output: "export",
  reactStrictMode: true,
  poweredByHeader: false,
  // 图标按需打包（plan 第 12 节：首次加载 JS < 200 KB）；组件库靠子路径拆分，见 packages/ui/src/index.ts
  experimental: { optimizePackageImports: ["lucide-react"] },
  transpilePackages: ["@akari/ui", "@akari/fees"],
};

export default config;
