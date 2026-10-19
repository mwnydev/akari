"use client";

import { Amount, Badge, Button, Card, CardTitle, Field, Textarea, type BadgeTone } from "@akari/ui";
import { ChatBox } from "@akari/ui/chat";
import { SegmentedControl } from "@akari/ui/tabs";
import { toast } from "@akari/ui/toast";
import { useState } from "react";

type Stage = "evidence" | "ruling" | "appeal" | "vote";

interface Case {
  id: string;
  order: string;
  title: string;
  amount: string;
  stage: Stage;
  due: string;
  dual: boolean;
  opener: "买家" | "卖家";
}

const cases: Case[] = [
  {
    id: "D-2041",
    order: "1001",
    title: "激活码提示已被使用",
    amount: "45000000",
    stage: "ruling",
    due: "还剩 31 小时",
    dual: false,
    opener: "买家",
  },
  {
    id: "D-2039",
    order: "0987",
    title: "代运维中途停止响应",
    amount: "300000000",
    stage: "ruling",
    due: "还剩 9 小时",
    dual: true,
    opener: "买家",
  },
  {
    id: "D-2036",
    order: "0962",
    title: "买家收货后拒绝确认",
    amount: "88000000",
    stage: "evidence",
    due: "举证还剩 40 小时",
    dual: false,
    opener: "卖家",
  },
  {
    id: "D-2030",
    order: "0911",
    title: "账号找回后被追回",
    amount: "520000000",
    stage: "vote",
    due: "投票还剩 50 小时",
    dual: true,
    opener: "买家",
  },
];

const stageLabel: Record<Stage, { label: string; tone: BadgeTone }> = {
  evidence: { label: "举证中", tone: "neutral" },
  ruling: { label: "待裁决", tone: "warning" },
  appeal: { label: "申诉复核", tone: "accent" },
  vote: { label: "公开投票", tone: "accent" },
};

const filters = [
  { value: "all", label: "全部" },
  { value: "ruling", label: "待裁决" },
  { value: "evidence", label: "举证中" },
  { value: "vote", label: "公开投票" },
] as const;

type Filter = (typeof filters)[number]["value"];

export function DisputeBoard() {
  const [filter, setFilter] = useState<Filter>("all");
  const [selected, setSelected] = useState("D-2041");
  const [toBuyer, setToBuyer] = useState(100);
  const list = cases.filter((c) => filter === "all" || c.stage === filter);
  const cur = cases.find((c) => c.id === selected) ?? cases[0];
  if (!cur) return null;
  // 只做显示用的比例拆分；真正的金额由合约按 bps 计算
  const buyerPart = (BigInt(cur.amount) * BigInt(toBuyer)) / 100n;

  return (
    <div className="grid gap-4 lg:grid-cols-[22rem_1fr]">
      <section className="min-w-0 space-y-3">
        <SegmentedControl
          label="按阶段筛选"
          options={[...filters]}
          value={filter}
          onValueChange={setFilter}
        />
        <Card className="p-0 sm:p-0">
          <ul className="divide-y divide-border">
            {list.map((c) => (
              <li key={c.id}>
                <button
                  type="button"
                  onClick={() => setSelected(c.id)}
                  aria-current={c.id === cur.id}
                  className="block w-full px-4 py-3 text-left hover:bg-surface-2 aria-[current=true]:bg-accent-soft"
                >
                  <span className="flex items-center justify-between gap-2">
                    <span className="amount text-sm text-fg-muted">
                      {c.id} · 订单 #{c.order}
                    </span>
                    <Badge tone={stageLabel[c.stage].tone}>{stageLabel[c.stage].label}</Badge>
                  </span>
                  <span className="mt-0.5 block font-medium">{c.title}</span>
                  <span className="flex items-center justify-between text-sm text-fg-muted">
                    <Amount value={c.amount} />
                    <span>{c.due}</span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </Card>
      </section>
      <section className="min-w-0 space-y-4">
        <Card>
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-xl font-semibold">{cur.title}</h2>
            <Badge tone={stageLabel[cur.stage].tone}>{stageLabel[cur.stage].label}</Badge>
            {cur.dual ? <Badge tone="warning">≥ 200 USDT · 双人独立裁决</Badge> : null}
          </div>
          <p className="mt-1 text-sm text-fg-muted">
            {cur.opener}发起 · 订单金额 <Amount value={cur.amount} /> · 当事人与你无关联、90
            天内无交易（已自动校验）
          </p>
        </Card>
        <div className="grid gap-4 xl:grid-cols-2">
          <Card className="space-y-2">
            <CardTitle>双方陈述与证据</CardTitle>
            <p className="text-sm">
              <Badge>买家</Badge> 激活码提示「许可证已被使用」，要求更换未果。附 3 张截图。
            </p>
            <p className="text-sm">
              <Badge>卖家</Badge> 发货时激活码有效，怀疑买家已激活后转卖。附发货记录。
            </p>
            <p className="text-xs text-fg-muted">交易帖快照哈希一致 · 聊天哈希链校验通过</p>
          </Card>
          <ChatBox
            readOnly
            header={<p className="font-medium">订单聊天（只读）</p>}
            messages={[
              { id: "1", kind: "system", text: "买家已付款", time: "10:02" },
              { id: "2", kind: "them", text: "卡密发了，激活遇到问题随时说", time: "10:03" },
              { id: "3", kind: "me", text: "提示许可证已被使用", time: "10:20" },
            ]}
          />
        </div>
        <Card className="space-y-4">
          <CardTitle>裁决意见</CardTitle>
          <div>
            <label htmlFor="ratio" className="mb-1 block text-sm font-medium">
              未领取金额判给买家 <span className="amount">{toBuyer}%</span>
            </label>
            <input
              id="ratio"
              type="range"
              min={0}
              max={100}
              step={5}
              value={toBuyer}
              onChange={(e) => setToBuyer(Number(e.target.value))}
              className="w-full accent-(--color-accent)"
            />
            <p className="amount mt-1 flex justify-between text-sm text-fg-muted">
              <span>
                买家 <Amount value={buyerPart} />
              </span>
              <span>
                卖家 <Amount value={BigInt(cur.amount) - buyerPart} />
              </span>
            </p>
          </div>
          <Field label="裁决理由（对双方公开，写入审计日志）">
            <Textarea placeholder="依据哪些证据、为什么这样分配" />
          </Field>
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => toast.success("裁决意见已提交，将进入今日仲裁 Safe 批量执行")}>
              提交裁决意见
            </Button>
            <Button variant="secondary">标记无法裁决（转公开投票）</Button>
          </div>
          <p className="text-xs text-fg-muted">
            裁决只是意见：链上执行由仲裁
            Safe（2/3，签名人当期不审案）每天批量完成，且只能分给买卖双方。
          </p>
        </Card>
      </section>
    </div>
  );
}
