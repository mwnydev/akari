import { Card, CardTitle } from "@akari/ui";
import type { Metadata } from "next";
import { UsernameForm } from "../../components/settings/UsernameForm";
import { AppShell } from "../../components/forum/AppShell";
import { safeNext } from "../../lib/safe-next";
import { requireMe } from "../../lib/session";

export const metadata: Metadata = { title: "账号设置", robots: { index: false } };

export default async function SettingsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const raw = (await searchParams).next;
  // 从钱包「开通交易」跳来设置用户名：保存后回去继续；登录后也回到本页（带着 next）
  const next = raw === undefined ? undefined : safeNext(raw);
  const me = await requireMe(next ? `/settings?next=${encodeURIComponent(next)}` : "/settings");
  return (
    <AppShell>
      <div className="max-w-xl space-y-4">
        <h1 className="text-2xl font-semibold">账号设置</h1>
        <Card className="space-y-3">
          <CardTitle>{me.username ? "修改用户名" : "设置用户名"}</CardTitle>
          <p className="text-sm text-fg-muted">
            用户名用于个人主页地址、@提及和交易；还没设置时别人看到的是「用户 {me.id}」。
          </p>
          {next && !me.username ? (
            <p className="rounded-md bg-warning-soft px-3 py-2 text-sm">
              开通交易前需要先设置用户名：交易对手和仲裁员都按用户名认人。保存后回去继续。
            </p>
          ) : null}
          <UsernameForm id={me.id} current={me.username ?? null} next={next} />
        </Card>
      </div>
    </AppShell>
  );
}
