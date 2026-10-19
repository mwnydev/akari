import {
  Amount,
  Avatar,
  Badge,
  Button,
  Card,
  CardTitle,
  EmptyState,
  ErrorState,
  Field,
  Input,
  LoadingState,
  NetworkBadge,
  OnceProgress,
  StreamProgress,
  Toolbar,
  UserCard,
} from "@akari/ui";
import { ChatBox } from "@akari/ui/chat";
import type { Metadata } from "next";
import type { ReactNode } from "react";
import { SiteNav } from "../../components/SiteNav";
import { Interactive } from "./Interactive";

export const metadata: Metadata = { title: "设计系统" };

// Storybook 与截图对比测试是后续任务（plan 第 14 节）；先用本页集中展示令牌与组件。

const colors = [
  "bg",
  "surface",
  "surface-2",
  "fg",
  "fg-muted",
  "accent",
  "accent-fill",
  "success",
  "warning",
  "danger",
  "online",
];

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card>
      <CardTitle className="mb-4">{title}</CardTitle>
      {children}
    </Card>
  );
}

export default function DesignPage() {
  return (
    <>
      <SiteNav active="/design" />
      <main className="mx-auto max-w-5xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">设计系统</h1>
        <p className="text-fg-muted">
          Liquid
          Glass：玻璃只用于导航、工具栏、弹窗；正文与金额实底高对比。系统开启「降低透明度」或「增强对比度」时玻璃退回实色。
        </p>
        <Section title="颜色令牌">
          <ul className="grid grid-cols-2 gap-3 sm:grid-cols-5">
            {colors.map((c) => (
              <li key={c} className="text-sm">
                <span
                  className="block h-12 rounded-md border border-border"
                  style={{ background: `var(--color-${c})` }}
                />
                <code className="text-fg-muted">--color-{c}</code>
              </li>
            ))}
          </ul>
        </Section>
        <Section title="字体、圆角、阴影、玻璃">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1">
              <p className="text-2xl font-semibold">标题 24 / 600</p>
              <p>正文 16px，行高 1.7。中文字体 PingFang SC / Noto Sans SC。</p>
              <p className="text-sm text-fg-muted">辅助文字 14px</p>
              <p className="text-xl">
                金额等宽数字：
                <Amount value="1234567890" />
              </p>
            </div>
            <div className="flex flex-wrap items-end gap-3">
              {(["sm", "md", "lg", "xl"] as const).map((r) => (
                <span
                  key={r}
                  className="flex size-16 items-center justify-center border border-border bg-surface-2 text-xs shadow-float"
                  style={{ borderRadius: `var(--radius-${r})` }}
                >
                  {r}
                </span>
              ))}
              <div className="relative h-24 w-48 overflow-hidden rounded-lg bg-[linear-gradient(135deg,var(--color-accent),var(--color-success))]">
                <div className="glass absolute inset-3 flex items-center justify-center rounded-md text-sm font-medium shadow-float">
                  玻璃材质
                </div>
              </div>
            </div>
          </div>
        </Section>
        <Section title="按钮、徽标、输入">
          <div className="flex flex-wrap gap-2">
            <Button>主要</Button>
            <Button variant="secondary">次要</Button>
            <Button variant="ghost">文字</Button>
            <Button variant="danger">发起争议</Button>
            <Button disabled>禁用</Button>
            <Button size="sm">小</Button>
            <Button size="lg">大按钮</Button>
          </div>
          <div className="mt-4 flex flex-wrap gap-2">
            <Badge>普通</Badge>
            <Badge tone="accent">Lv5</Badge>
            <Badge tone="success">已放款</Badge>
            <Badge tone="warning">争议中</Badge>
            <Badge tone="danger">已封禁</Badge>
            <NetworkBadge />
          </div>
          <div className="mt-4 grid gap-3 sm:grid-cols-2">
            <Field label="提币地址" hint="新地址 24 小时后生效，期间可撤销">
              <Input placeholder="0x…" />
            </Field>
            <Field label="金额">
              <Input inputMode="decimal" placeholder="0.00" className="amount" />
            </Field>
          </div>
        </Section>
        <Section title="交互：分段控件、签名确认弹窗、提示条">
          <Interactive />
        </Section>
        <Section title="头像与用户名片">
          <div className="flex flex-wrap items-center gap-4">
            <Avatar name="小林" online />
            <Avatar name="阿青" online={false} />
            <Avatar name="momo" size={56} />
          </div>
          <div className="mt-4 grid gap-4 sm:grid-cols-2">
            <UserCard
              name="小林"
              level={5}
              levelName="老会员"
              online
              reputation={{ successPermille: 986, deals: 214, disputesLost: 1, replyMinutes: 10 }}
            />
            <UserCard
              name="阿青"
              level={3}
              online={false}
              lastSeen="5 分钟前"
              reputation={{ successPermille: 1000, deals: 3, disputesLost: 0 }}
            />
          </div>
        </Section>
        <Section title="订单进度">
          <div className="grid gap-6 sm:grid-cols-2">
            <OnceProgress state="funded" hint="发货期限还剩 2 天 23 小时" />
            <OnceProgress state="delivered" hint="确认期还剩 6 天，期满自动放款" />
            <OnceProgress state="released" />
            <OnceProgress state="disputed" delivered hint="举证期还剩 48 小时" />
            <StreamProgress
              periodLabel="每天 10 USDT"
              periods={30}
              elapsed={12}
              net="296900000"
              unlocked="118760000"
              claimed="108863333"
            />
            <StreamProgress
              periodLabel="每天 10 USDT"
              periods={30}
              elapsed={18}
              net="296900000"
              unlocked="178140000"
              claimed="168243333"
              disputed
            />
          </div>
        </Section>
        <Section title="聊天框">
          <ChatBox
            typing
            messages={[
              { id: "1", kind: "system", text: "买家已付款 45.050000 USDT", time: "10:02" },
              { id: "2", kind: "them", text: "卡密已经自动发了，激活有问题随时说", time: "10:03" },
              { id: "3", kind: "me", text: "收到，正在激活", time: "10:05", read: true },
            ]}
          />
        </Section>
        <Section title="空 / 加载 / 出错">
          <div className="grid gap-4 sm:grid-cols-3">
            <EmptyState
              title="还没有订单"
              description="去交易板块看看"
              action={<Button size="sm">逛逛</Button>}
            />
            <LoadingState />
            <ErrorState
              message="网络有点慢，没能读取到链上状态"
              action={
                <Button size="sm" variant="secondary">
                  重试
                </Button>
              }
            />
          </div>
        </Section>
        <Section title="浮动工具栏">
          <Toolbar className="w-fit">
            <Button variant="ghost" size="sm">
              回复
            </Button>
            <Button variant="ghost" size="sm">
              收藏
            </Button>
            <Button variant="ghost" size="sm">
              分享
            </Button>
          </Toolbar>
        </Section>
      </main>
    </>
  );
}
