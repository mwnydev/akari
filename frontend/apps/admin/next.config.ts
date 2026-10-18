import type { NextConfig } from "next";

const config: NextConfig = {
  output: "export",
  reactStrictMode: true,
  poweredByHeader: false,
  transpilePackages: ["@akari/ui"],
};

export default config;
