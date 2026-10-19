"use client";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";

// 页面里的交互岛（编辑器、弹窗、登录表单）在浏览器里共用一个客户端；服务端渲染每次新建，不跨请求共享。
let browserClient: QueryClient | undefined;

function makeClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: 1, staleTime: 30_000 }, mutations: { retry: false } },
  });
}

export function QueryProvider({ children }: { children: ReactNode }) {
  const [client] = useState(() =>
    typeof window === "undefined" ? makeClient() : (browserClient ??= makeClient()),
  );
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
