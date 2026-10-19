import { Button, Card, EmptyState } from "@akari/ui";
import Link from "next/link";
import { AppShell } from "../components/forum/AppShell";

export default function NotFound() {
  return (
    <AppShell>
      <div className="max-w-3xl">
        <Card>
          <EmptyState
            title="内容不存在或已被删除"
            description="可能是链接有误，或主题已被隐藏、板块需要更高等级才能查看。"
            action={
              <Button asChild variant="secondary" size="sm">
                <Link href="/">返回首页</Link>
              </Button>
            }
          />
        </Card>
      </div>
    </AppShell>
  );
}
