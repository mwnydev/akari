import type { Metadata } from "next";
import type { ReactNode } from "react";

export const metadata: Metadata = {
  title: { default: "仲裁", template: "%s · 仲裁 · Akari" },
  robots: { index: false },
};

export default function ArbitrationLayout({ children }: { children: ReactNode }) {
  return children;
}
