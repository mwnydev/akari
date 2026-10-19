import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";

// 整组不读会话、不调接口，构建时直接生成静态页
export const dynamic = "force-static";

export const metadata: Metadata = {
  title: { default: "设计稿 v2", template: "%s · 设计稿 v2" },
  robots: { index: false },
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#ffffff" },
    { media: "(prefers-color-scheme: dark)", color: "#1f1f1f" },
  ],
};

/**
 * 设计稿 v2：只用模拟数据、不调接口，整组路由静态渲染，供站长确认后再改正式页面。
 * data-forum-theme 打开 globals.css 里的论坛扁平主题，只作用于这组页面。
 */
export default function DesignV2Layout({ children }: { children: ReactNode }) {
  return <div data-forum-theme="flat">{children}</div>;
}
