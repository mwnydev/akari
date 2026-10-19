"use client";

import { ApiError, errorMessage, type Schemas } from "@akari/api-client";
import { Amount, Button, Field, Input, Textarea } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { useForm } from "react-hook-form";
import * as z from "zod/mini";
import {
  allocate,
  bpsLabel,
  disputedPrincipal,
  intentMatches,
  orderBond,
  parseIntent,
  percentToBps,
  type SignedIntent,
} from "../../lib/ruling";
import { rulingOptions, signAndSubmit, type RulingRequest } from "../../lib/ruling-passkey";
import { RULING_KIND } from "../../lib/arbitration";
import { QueryProvider } from "../QueryProvider";
import RulingPasskeySetup from "./RulingPasskeySetup";

const percent = z
  .string()
  .check(z.refine((v) => percentToBps(v) !== null, { error: "请输入 0–100，最多两位小数" }));

const Schema = z.object({
  toBuyer: percent,
  bondToBuyer: percent,
  reason: z.string().check(
    z.refine((v) => v.trim().length > 0, { error: "必须写公开理由" }),
    z.maxLength(2000, { error: "理由最多 2000 字" }),
  ),
});
type Input = z.infer<typeof Schema>;

const OUTCOME: Record<string, string> = {
  waiting: "已提交。两人裁决：等另一位仲裁员提交后才会比对与公示。",
  disagreed: "两人意见不一致，已转站长终裁。",
  published: "裁决已公示，进入 72 小时申诉期；无人申诉才进入仲裁 Safe 执行。",
  final: "复核结果为最终裁决，等待仲裁 Safe 执行。",
};

const PRESETS = [
  { label: "全额退买家", toBuyer: "100" },
  { label: "各一半", toBuyer: "50" },
  { label: "全额给卖家", toBuyer: "0" },
];

interface Props {
  caseId: number;
  kind: Schemas["RulingKind"];
  order: Schemas["OrderView"];
  /** 举证期还没结束（提交时服务端会拒绝，先提示） */
  evidenceOpen: boolean;
}

interface Pending {
  body: RulingRequest;
  intent: SignedIntent;
  options: unknown;
}

function Form({ caseId, kind, order, evidenceOpen }: Props) {
  const router = useRouter();
  const principal = disputedPrincipal(order);
  const bond = orderBond(order);
  const form = useForm<Input>({
    resolver: standardSchemaResolver(Schema),
    defaultValues: { toBuyer: "100", bondToBuyer: "0", reason: "" },
  });
  const [pending, setPending] = useState<Pending | null>(null);
  const [needPasskey, setNeedPasskey] = useState(false);
  const [outcome, setOutcome] = useState<string | null>(null);

  const toBuyerBps = percentToBps(form.watch("toBuyer"));
  const bondBps = bond > 0n ? percentToBps(form.watch("bondToBuyer")) : 0;
  const estimate =
    principal !== null && toBuyerBps !== null && bondBps !== null
      ? allocate(principal, bond, toBuyerBps, bondBps)
      : null;

  // 第一步：服务端校验并算出签署内容（金额以它为准）
  const prepare = useMutation({
    mutationFn: async (v: Input) => {
      const body: RulingRequest = {
        kind,
        to_buyer_bps: percentToBps(v.toBuyer) ?? 0,
        bond_to_buyer_bps: bond > 0n ? (percentToBps(v.bondToBuyer) ?? 0) : 0,
        reason: v.reason.trim(),
      };
      const res = await rulingOptions(caseId, body);
      const intent = parseIntent(res.intent);
      if (
        !intent ||
        !intentMatches(intent, {
          caseId,
          kind,
          toBuyerBps: body.to_buyer_bps,
          bondToBuyerBps: body.bond_to_buyer_bps,
        })
      ) {
        throw new ApiError("RULING_MISMATCH", "服务端返回的签署内容与表单不一致，已停止", 0);
      }
      return { body, intent, options: res.options };
    },
    onSuccess: (p) => {
      setNeedPasskey(false);
      setPending(p);
    },
    onError: (e) => setNeedPasskey(e instanceof ApiError && e.code === "PASSKEY_REQUIRED"),
  });

  // 第二步：裁决 Passkey 签署并提交同一份裁决
  const sign = useMutation({
    mutationFn: (p: Pending) => signAndSubmit(caseId, p.body, p.options),
    onSuccess: (r) => {
      setPending(null);
      setOutcome(OUTCOME[r.outcome] ?? "裁决已提交");
      router.refresh();
    },
    // 服务端收到提交就作废这次确认（无论成败）；只有系统弹窗被取消时才能用同一份确认重试
    onError: (e) => {
      if (e instanceof ApiError && e.status !== 0) setPending(null);
    },
  });

  if (outcome) {
    return (
      <p role="status" className="rounded-md bg-success-soft px-3 py-2 text-sm text-success">
        {outcome}
      </p>
    );
  }

  const errors = form.formState.errors;
  return (
    <form
      className="space-y-4"
      noValidate
      aria-label="裁决"
      onSubmit={form.handleSubmit((v) => {
        sign.reset();
        prepare.mutate(v);
      })}
    >
      <div className="space-y-2">
        <Field
          label="争议本金判给买家（%）"
          hint="其余归卖家；按合约取整：卖家份额向下取整，余数归买家"
          error={errors.toBuyer?.message}
        >
          <Input inputMode="decimal" {...form.register("toBuyer")} />
        </Field>
        <div className="flex flex-wrap gap-2">
          {PRESETS.map((p) => (
            <Button
              key={p.label}
              size="sm"
              variant="secondary"
              onClick={() => form.setValue("toBuyer", p.toBuyer, { shouldValidate: true })}
            >
              {p.label}
            </Button>
          ))}
        </div>
      </div>
      {bond > 0n ? (
        <Field
          label="锁定的卖家保证金赔给买家（%）"
          hint="判卖家责任时可把随订单锁定的保证金按比例赔给买家"
          error={errors.bondToBuyer?.message}
        >
          <Input inputMode="decimal" {...form.register("bondToBuyer")} />
        </Field>
      ) : null}
      <div className="rounded-md bg-surface-2 px-3 py-2 text-sm" data-testid="ruling-estimate">
        {estimate ? (
          <>
            预估：买家 <Amount value={estimate.principalToBuyer} /> · 卖家{" "}
            <Amount value={estimate.principalToSeller} />
            {bond > 0n ? (
              <>
                {" "}
                · 保证金赔给买家 <Amount value={estimate.bondToBuyer} />
              </>
            ) : null}
          </>
        ) : (
          "分期订单的争议金额（未领取的可解锁本金）由服务端按链上订单计算，确认时显示。"
        )}
        <span className="block text-xs text-fg-muted">
          不含手续费、gas 与争议费的结算；签署前以服务端金额为准。
        </span>
      </div>
      <Field label="公开理由（对双方公开，写入审计日志）" error={errors.reason?.message}>
        <Textarea
          maxLength={2000}
          placeholder="依据哪些证据、为什么这样分配"
          {...form.register("reason")}
        />
      </Field>
      {evidenceOpen ? (
        <p className="text-sm text-warning">举证期还没结束，结束后才能提交裁决。</p>
      ) : null}
      {needPasskey ? (
        <RulingPasskeySetup onDone={() => setNeedPasskey(false)} />
      ) : prepare.isError || (sign.isError && !pending) ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(prepare.error ?? sign.error)}
        </p>
      ) : null}
      <Button type="submit" disabled={prepare.isPending}>
        {prepare.isPending ? "校验中…" : `确认${RULING_KIND[kind]}`}
      </Button>
      {pending ? (
        <Dialog
          open
          onOpenChange={(o) => {
            if (!o && !sign.isPending) setPending(null);
          }}
        >
          <DialogContent
            title="用裁决 Passkey 签署"
            description="Passkey 签署的是下面这份裁决（含服务端按链上订单算出的金额与理由哈希），内容改动即失效。"
          >
            <dl
              className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm"
              data-testid="ruling-intent"
            >
              <dt className="text-fg-muted">类型</dt>
              <dd>{RULING_KIND[pending.intent.kind]}</dd>
              <dt className="text-fg-muted">本金判给买家</dt>
              <dd>
                {bpsLabel(pending.intent.toBuyerBps)} ·{" "}
                <Amount value={pending.intent.principalToBuyer} />
              </dd>
              <dt className="text-fg-muted">本金判给卖家</dt>
              <dd>
                <Amount value={pending.intent.principalToSeller} />
              </dd>
              <dt className="text-fg-muted">保证金赔给买家</dt>
              <dd>
                {bpsLabel(pending.intent.bondToBuyerBps)} ·{" "}
                <Amount value={pending.intent.bondToBuyer} />
              </dd>
            </dl>
            {sign.isError ? (
              <p role="alert" className="mt-3 text-sm text-danger">
                {errorMessage(sign.error)}
              </p>
            ) : null}
            <div className="mt-4 flex justify-end gap-2">
              <Button
                variant="secondary"
                onClick={() => setPending(null)}
                disabled={sign.isPending}
              >
                返回修改
              </Button>
              <Button
                onClick={() => {
                  prepare.reset();
                  sign.mutate(pending);
                }}
                disabled={sign.isPending}
              >
                <KeyRound size={16} aria-hidden />
                {sign.isPending ? "等待 Passkey…" : "签署并提交"}
              </Button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}
    </form>
  );
}

/** 裁决表单：比例 → 预估金额 → 服务端算出签署内容 → 裁决 Passkey 签署提交。 */
export default function RulingForm(props: Props) {
  return (
    <QueryProvider>
      <Form {...props} />
    </QueryProvider>
  );
}
