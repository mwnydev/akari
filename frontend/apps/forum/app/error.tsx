"use client";

import { Button, Card, ErrorState } from "@akari/ui";

/** 渲染期间的意外错误（接口错误由各页面自己显示中文提示，走到这里的是程序缺陷或网络中断）。 */
export default function RouteError({ reset }: { error: Error; reset: () => void }) {
  return (
    <main className="mx-auto max-w-3xl px-4 py-5">
      <Card>
        <ErrorState
          message="页面出了点问题，请稍后重试"
          action={
            <Button variant="secondary" size="sm" onClick={reset}>
              重试
            </Button>
          }
        />
      </Card>
    </main>
  );
}
