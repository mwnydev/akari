import { Button, Card, EmptyState } from "@akari/ui";
import type { Metadata } from "next";

export const metadata: Metadata = { title: "页面不存在" };

export default function NotFound() {
  return (
    <main className="mx-auto max-w-xl px-4 py-16">
      <Card>
        <EmptyState
          title="后台没有这个页面"
          description="链接可能有误，从后台首页的导航进入。"
          action={
            <Button asChild variant="secondary" size="sm">
              <a href="/">回后台首页</a>
            </Button>
          }
        />
      </Card>
    </main>
  );
}
