import { Avatar, Badge, Button, Card, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import { SiteNav } from "../../../components/SiteNav";
import { userName } from "../../../lib/display";
import { WALLET_ORIGIN } from "../../../lib/public-config";
import { getMe } from "../../../lib/session";

type Params = Promise<{ name: string }>;

/** 路由参数可能仍是 URL 编码形式；本身含 % 的名字解码会失败，原样使用。 */
function decodeParam(raw: string): string {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/** 路由参数是用户名，或还没设置用户名的用户的数字 ID（见 lib/display profileHref）。 */
function displayOf(name: string): string {
  return /^\d+$/.test(name) ? userName({ id: Number(name) }) : name;
}

export async function generateMetadata({ params }: { params: Params }): Promise<Metadata> {
  return { title: displayOf(decodeParam((await params).name)), robots: { index: false } };
}

/**
 * 个人主页。后端还没有公开的用户资料、帖子列表与交易信誉接口，先显示名字；
 * 本人访问时显示自己的账号信息和开通交易入口。
 */
export default async function ProfilePage({ params }: { params: Params }) {
  const raw = decodeParam((await params).name);
  const me = await getMe();
  const self = me !== null && (raw === me.username || raw === String(me.id));
  const name = self ? userName(me) : displayOf(raw);
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-3xl space-y-4 px-4 py-5">
        <Card className="flex flex-col items-center gap-3 text-center sm:flex-row sm:text-left">
          <Avatar name={name} size={80} />
          <div className="min-w-0 flex-1">
            <h1 className="flex flex-wrap items-center justify-center gap-2 text-2xl font-semibold sm:justify-start">
              {name}
              {self ? <Badge tone="accent">这是你</Badge> : null}
            </h1>
            {self ? (
              <p className="text-sm text-fg-muted">登录邮箱：{me.email}（仅自己可见）</p>
            ) : null}
          </div>
          {self ? (
            <Button asChild variant="secondary" size="sm">
              <a href={`${WALLET_ORIGIN}/setup`}>开通交易 / 管理钱包</a>
            </Button>
          ) : null}
        </Card>
        <Card>
          <EmptyState
            title="个人主页即将开放"
            description="主题、回复与交易信誉（成功率、成交笔数、争议败诉、回复用时）会显示在这里。"
          />
        </Card>
      </main>
    </>
  );
}
