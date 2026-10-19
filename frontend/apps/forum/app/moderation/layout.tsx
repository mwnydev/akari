import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: { default: "管理", template: "%s · 管理 · Akari" },
  robots: { index: false },
};

export default function ModerationLayout({ children }: { children: ReactNode }) {
  return children;
}
