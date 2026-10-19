import { Amount, Avatar, Badge, Card, EmptyState, reputationLine } from "@akari/ui";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@akari/ui/tabs";
import { notFound } from "next/navigation";
import { SiteNav } from "../../../components/SiteNav";
import { TopicRow } from "../../../components/TopicRow";
import { topics, users } from "../../../lib/mock";

export default async function ProfilePage({ params }: { params: Promise<{ name: string }> }) {
  const { name } = await params;
  const u = users[decodeURIComponent(name)];
  if (!u) notFound();
  const own = topics.filter((t) => t.author === u.name);
  const r = u.reputation;
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-3xl space-y-4 px-4 py-5">
        <Card className="flex flex-col items-center gap-3 text-center sm:flex-row sm:text-left">
          <Avatar name={u.name} size={80} online={u.online} />
          <div className="min-w-0 flex-1">
            <h1 className="flex flex-wrap items-center justify-center gap-2 text-2xl font-semibold sm:justify-start">
              {u.name}
              <Badge tone="accent">
                Lv{u.level} {u.levelName}
              </Badge>
              {r && r.deals >= 10 ? <Badge tone="success">认证卖家 · 保证金 500</Badge> : null}
            </h1>
            <p className="text-sm text-fg-muted">
              {u.online ? "在线" : `最近活跃：${u.lastSeen ?? "很久以前"}`} · 注册于 {u.joined}
            </p>
            {u.signature ? <p className="mt-1">{u.signature}</p> : null}
          </div>
        </Card>
        {r ? (
          <Card>
            <h2 className="mb-3 font-semibold">交易信誉</h2>
            <dl className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              {[
                [
                  "成功率",
                  r.deals < 10
                    ? "—"
                    : `${Math.floor(r.successPermille / 10)}.${r.successPermille % 10}%`,
                ],
                ["成交", `${r.deals} 笔`],
                ["争议败诉", `${r.disputesLost} 次`],
                ["平均回复", r.replyMinutes !== undefined ? `${r.replyMinutes} 分钟` : "—"],
              ].map(([k, v]) => (
                <div key={k} className="rounded-lg bg-surface-2 p-3">
                  <dt className="text-sm text-fg-muted">{k}</dt>
                  <dd className="amount text-xl font-semibold">{v}</dd>
                </div>
              ))}
            </dl>
            <p className="mt-3 text-sm text-fg-muted">
              {reputationLine(r)}。只统计链上订单结果，每日从链上重算。保证金可用余额{" "}
              <Amount value="500000000" />
            </p>
          </Card>
        ) : null}
        <Tabs defaultValue="topics">
          <TabsList aria-label="个人内容">
            <TabsTrigger value="topics">主题</TabsTrigger>
            <TabsTrigger value="replies">回复</TabsTrigger>
            <TabsTrigger value="reviews">交易评价</TabsTrigger>
          </TabsList>
          <TabsContent value="topics" className="mt-3">
            <Card className="p-0 sm:p-0">
              {own.length ? (
                <ul className="divide-y divide-border">
                  {own.map((t) => (
                    <TopicRow key={t.id} t={t} />
                  ))}
                </ul>
              ) : (
                <EmptyState title="还没有发过主题" />
              )}
            </Card>
          </TabsContent>
          <TabsContent value="replies" className="mt-3">
            <Card>
              <EmptyState title="还没有回复" />
            </Card>
          </TabsContent>
          <TabsContent value="reviews" className="mt-3">
            <Card>
              <EmptyState title="暂无评价" description="完成担保交易后，双方可以互评" />
            </Card>
          </TabsContent>
        </Tabs>
      </main>
    </>
  );
}
