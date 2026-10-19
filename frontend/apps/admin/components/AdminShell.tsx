"use client";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { AdminNav } from "./AdminNav";
import { SessionBanner } from "./SessionBanner";
import { SessionProvider } from "./session/SessionProvider";

/** 每个后台页面的外壳：查询缓存、后台会话（未登录显示登录页）、导航与到期提醒。 */
export function AdminShell({
  active,
  title,
  children,
}: {
  active: string;
  title: string;
  children: ReactNode;
}) {
  const [client] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: { retry: false, staleTime: 15_000 },
          mutations: { retry: false },
        },
      }),
  );
  return (
    <QueryClientProvider client={client}>
      <SessionProvider>
        <AdminNav active={active} />
        <SessionBanner />
        <main className="mx-auto max-w-6xl space-y-4 px-4 py-5">
          <h1 className="text-2xl font-semibold">{title}</h1>
          {children}
        </main>
      </SessionProvider>
    </QueryClientProvider>
  );
}
