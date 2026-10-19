import { Badge, Card, CardTitle } from "@akari/ui";
import { ShieldCheck } from "lucide-react";
import { SiteNav } from "../components/SiteNav";
import { TopicRow } from "../components/TopicRow";
import { boards, topics } from "../lib/mock";

export default function Home() {
  return (
    <>
      <SiteNav active="/" />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-[1fr_18rem]">
        <section className="min-w-0 space-y-4">
          <Card className="flex items-start gap-3 bg-accent-soft">
            <ShieldCheck className="mt-0.5 shrink-0 text-accent" aria-hidden />
            <div>
              <p className="font-medium">链上担保，平台转不走你的钱</p>
              <p className="text-sm text-fg-muted">
                USDT 托管在 Arbitrum One 合约里，确认收货才放款；gas 平台代付，钱包无需 ETH。
              </p>
            </div>
          </Card>
          <Card className="p-0 sm:p-0">
            <div className="flex items-center justify-between px-4 pt-4">
              <CardTitle>最新主题</CardTitle>
            </div>
            <ul className="divide-y divide-border">
              {topics.map((t) => (
                <TopicRow key={t.id} t={t} />
              ))}
            </ul>
          </Card>
        </section>
        <aside className="min-w-0">
          <Card className="p-0 sm:p-0">
            <CardTitle className="px-4 pt-4">板块</CardTitle>
            <ul className="divide-y divide-border">
              {boards.map((b) => (
                <li key={b.slug}>
                  <a href={`/f/${b.slug}`} className="block px-4 py-3 hover:bg-surface-2">
                    <span className="flex items-center gap-2 font-medium">
                      {b.name}
                      {b.trade ? <Badge tone="warning">担保</Badge> : null}
                    </span>
                    <span className="block text-sm text-fg-muted">{b.description}</span>
                    <span className="amount block text-xs text-fg-muted">
                      主题 {b.topics.toLocaleString("zh-CN")} · 今日 {b.today}
                    </span>
                  </a>
                </li>
              ))}
            </ul>
          </Card>
        </aside>
      </main>
    </>
  );
}
