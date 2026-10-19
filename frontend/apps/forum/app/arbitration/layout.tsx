import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: { default: "仲裁", template: "%s · 仲裁 · Akari" },
  robots: { index: false },
};

/** 仲裁页面用论坛扁平主题（globals.css 的 data-forum-theme）。 */
export default function ArbitrationLayout({ children }: { children: ReactNode }) {
  return <div data-forum-theme="flat">{children}</div>;
}
