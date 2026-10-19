import { errorMessage } from "@akari/api-client";
import { Button, Card, ErrorState } from "@akari/ui";
import Link from "next/link";

/** 整页加载失败（非 404）：中文提示 + 返回首页。放在 AppShell 的正文区里。 */
export function PageError({ error, title = "页面加载失败" }: { error: unknown; title?: string }) {
  return (
    <div className="max-w-3xl">
      <Card>
        <ErrorState
          title={title}
          message={errorMessage(error)}
          action={
            <Button asChild variant="secondary" size="sm">
              <Link href="/">返回首页</Link>
            </Button>
          }
        />
      </Card>
    </div>
  );
}
