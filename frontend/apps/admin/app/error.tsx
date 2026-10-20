"use client";

import { RouteError } from "@akari/ui/route-error";

export default function ErrorPage({ reset }: { error: Error; reset: () => void }) {
  return <RouteError title="页面出错 · Akari 后台" home="回后台首页" reset={reset} />;
}
