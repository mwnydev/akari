import type { Metadata } from "next";
import type { ReactNode } from "react";
import { PreviewProvider } from "../../../components/forum/preview";

// 整组不读会话、不调接口，构建时直接生成静态页
export const dynamic = "force-static";

export const metadata: Metadata = {
  title: { default: "设计稿 v2", template: "%s · 设计稿 v2" },
  robots: { index: false },
};

/**
 * 设计稿 v2（站长已确认）：用正式页面的组件渲染模拟数据，作为活的样式指南。整组路由静态渲染；
 * PreviewProvider 让组件里的写操作只改页面状态、不请求接口。
 */
export default function DesignV2Layout({ children }: { children: ReactNode }) {
  return <PreviewProvider>{children}</PreviewProvider>;
}
