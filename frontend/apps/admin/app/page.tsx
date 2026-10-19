import { Button, Card, CardTitle } from "@akari/ui";
import { AdminNav } from "../components/AdminNav";

export default function AdminHome() {
  return (
    <>
      <AdminNav active="/" />
      <main className="mx-auto max-w-5xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">概览</h1>
        <Card>
          <CardTitle>待处理</CardTitle>
          <p className="mt-1 text-fg-muted">争议 4 件 · 举报 17 条 · 证据脱敏 3 件</p>
          <Button asChild className="mt-3">
            <a href="/disputes">去争议面板</a>
          </Button>
        </Card>
      </main>
    </>
  );
}
