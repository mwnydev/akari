"use client";

import { RouteError } from "@akari/ui/route-error";

export default function ErrorPage({ reset }: { error: Error; reset: () => void }) {
  return <RouteError title="页面出错 · Akari 钱包" home="回钱包首页" reset={reset} />;
}
