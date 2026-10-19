import { Amount, Badge, Button, Card, CardTitle, Field, Textarea } from "@akari/ui";
import { ChatBox } from "@akari/ui/chat";
import { FileImage, Upload } from "lucide-react";
import type { Metadata } from "next";
import { WalletNav } from "../../../../components/WalletNav";
import { chat, checkoutCost, orderIds } from "../../../../lib/mock";

export const metadata: Metadata = { title: "争议审理" };

export function generateStaticParams() {
  return orderIds.map((id) => ({ id }));
}

const steps = [
  { label: "举证期", detail: "发起后 72 小时", state: "current" },
  { label: "仲裁员裁决", detail: "举证结束后 48 小时内；≥ 200 USDT 双人独立裁决", state: "todo" },
  { label: "申诉复核", detail: "任一方 72 小时内可申诉一次", state: "todo" },
  { label: "公开投票（仅无法裁决时）", detail: "交易过的账号投票 72 小时", state: "todo" },
  { label: "仲裁 Safe 执行", detail: "多签只能把钱分给买卖双方", state: "todo" },
] as const;

export default async function DisputePage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const c = checkoutCost();
  return (
    <>
      <WalletNav />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-[1fr_24rem]">
        <section className="min-w-0 space-y-4">
          <div>
            <p className="text-sm text-fg-muted">订单 #{id}</p>
            <h1 className="flex items-center gap-2 text-2xl font-semibold">
              争议审理 <Badge tone="warning">举证中</Badge>
            </h1>
            <p className="text-sm text-fg-muted">
              你已预付争议费 <Amount value={c.disputeFee} />
              ：胜诉全额退回，败诉归平台；按比例裁决按比例结算。
            </p>
          </div>
          <Card>
            <CardTitle className="mb-3">进度</CardTitle>
            <ol className="space-y-3">
              {steps.map((s, i) => (
                <li key={s.label} className="flex gap-3">
                  <span
                    className={
                      s.state === "current"
                        ? "flex size-6 shrink-0 items-center justify-center rounded-full bg-accent-fill text-sm text-accent-fg"
                        : "flex size-6 shrink-0 items-center justify-center rounded-full bg-surface-2 text-sm text-fg-muted"
                    }
                  >
                    {i + 1}
                  </span>
                  <div>
                    <p className={s.state === "current" ? "font-medium" : "text-fg-muted"}>
                      {s.label}
                      {s.state === "current" ? (
                        <span className="amount ml-2 text-sm text-warning">还剩 61:42:10</span>
                      ) : null}
                    </p>
                    <p className="text-sm text-fg-muted">{s.detail}</p>
                  </div>
                </li>
              ))}
            </ol>
          </Card>
          <Card className="space-y-3">
            <CardTitle>我的陈述与证据</CardTitle>
            <Field label="陈述" hint="仲裁员只看订单聊天、交易帖快照和双方证据">
              <Textarea defaultValue="卖家发的激活码提示「许可证已被使用」，要求更换未果，还让我线下转账退款。" />
            </Field>
            <ul className="grid grid-cols-3 gap-2">
              {["激活报错截图", "许可证页面", "聊天截图"].map((n) => (
                <li
                  key={n}
                  className="flex aspect-square flex-col items-center justify-center gap-1 rounded-lg bg-surface-2 p-2 text-center text-xs text-fg-muted"
                >
                  <FileImage size={24} aria-hidden />
                  {n}
                </li>
              ))}
            </ul>
            <div className="flex flex-wrap gap-2">
              <Button variant="secondary">
                <Upload size={18} aria-hidden />
                上传图片
              </Button>
              <Button>提交证据</Button>
            </div>
            <p className="text-xs text-fg-muted">
              证据图片仅双方与仲裁员可见，签名访问且有时效；结案后脱敏归入案例库。
            </p>
          </Card>
        </section>
        <ChatBox
          messages={chat}
          readOnly
          header={<p className="font-medium">订单聊天记录（哈希链已校验）</p>}
        />
      </main>
    </>
  );
}
