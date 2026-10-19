import { Card, CardTitle } from "@akari/ui";
import type { Metadata } from "next";
import { UsernameForm } from "../../components/settings/UsernameForm";
import { AppShell } from "../../components/forum/AppShell";
import { requireMe } from "../../lib/session";

export const metadata: Metadata = { title: "账号设置", robots: { index: false } };

export default async function SettingsPage() {
  const me = await requireMe("/settings");
  return (
    <AppShell>
      <div className="max-w-xl space-y-4">
        <h1 className="text-2xl font-semibold">账号设置</h1>
        <Card className="space-y-3">
          <CardTitle>{me.username ? "修改用户名" : "设置用户名"}</CardTitle>
          <p className="text-sm text-fg-muted">
            用户名用于个人主页地址、@提及和交易；还没设置时别人看到的是「用户 {me.id}」。
          </p>
          <UsernameForm id={me.id} current={me.username ?? null} />
        </Card>
      </div>
    </AppShell>
  );
}
