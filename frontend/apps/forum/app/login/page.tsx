import { Card } from "@akari/ui";
import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { SiteNav } from "../../components/SiteNav";
import { getMe } from "../../lib/session";
import { LoginForm } from "./LoginForm";
import { safeNext } from "./safe-next";

export const metadata: Metadata = { title: "登录", robots: { index: false } };

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const next = safeNext((await searchParams).next);
  if (await getMe()) redirect(next);

  return (
    <>
      <SiteNav />
      <main className="mx-auto mt-8 w-full max-w-md px-4">
        <Card className="p-6">
          <h1 className="text-2xl font-semibold">登录 / 注册</h1>
          <p className="mt-1 text-sm text-fg-muted">
            用邮箱验证码登录，无需密码；新邮箱会自动注册。
          </p>
          <LoginForm next={next} />
        </Card>
      </main>
    </>
  );
}
