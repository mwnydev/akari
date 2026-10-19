import { Toaster } from "@akari/ui/toast";
import type { Metadata } from "next";
import type { ReactNode } from "react";
import "./globals.css";

export const metadata: Metadata = {
  title: { default: "Akari 后台", template: "%s · Akari 后台" },
  robots: { index: false },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="zh-CN">
      <body className="min-h-dvh pb-10">
        {children}
        <Toaster position="top-center" />
      </body>
    </html>
  );
}
