"use client";

import { ApiError, errorMessage, type Schemas } from "@akari/api-client";
import { Button, Field, Input, formatUsdt } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import type { AuctionParams } from "../../lib/auction";
import { api, unwrap } from "../../lib/browser-api";
import { FEE_PAYERS } from "../../lib/fees";
import { walletAuctionSignHref } from "../../lib/links";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { usePostCaptcha } from "../captcha/PostCaptcha";
import { EditorField, useForumEditor } from "../composer/Editor";
import { QueryProvider } from "../QueryProvider";
import {
  ELIGIBILITY_OPTIONS,
  auctionPrices,
  auctionSchema,
  auctionTerms,
  durationOptions,
  minIncrementHint,
  pricesUsdt,
  type AuctionInput,
} from "./auction-schema";
import { WalletLink } from "../settings/UsernameGate";

const DAY = 86_400;
const DEFAULT_DURATION = 3 * DAY;
const selectClass = "h-10 w-full rounded-md border border-border bg-surface px-3 text-fg";

interface Props {
  forumSlug: string;
  trade: Schemas["TradeParams"];
  auction: AuctionParams;
}

function Form({ forumSlug, trade, auction }: Props) {
  const limits = useMemo(() => ({ ...trade, auction }), [trade, auction]);
  const schema = useMemo(() => auctionSchema(limits), [limits]);
  const durations = durationOptions(auction);
  const d = trade.defaults;
  const form = useForm<AuctionInput>({
    resolver: standardSchemaResolver(schema),
    defaultValues: {
      title: "",
      currency: "USDT",
      cnyRate: "",
      start: "",
      increment: "",
      reserve: "",
      duration: String(
        durations.some((o) => o.value === String(DEFAULT_DURATION))
          ? DEFAULT_DURATION
          : (durations[0]?.value ?? ""),
      ),
      eligibility: "none",
      eligibilityAmount: "",
      instant: false,
      shipDays: String(Math.round(d.ship_window / DAY)),
      confirmDays: String(Math.round(d.confirm_window / DAY)),
      feePayer: String(d.fee_payer) as AuctionInput["feePayer"],
    },
  });
  const [bodyError, setBodyError] = useState<string | null>(null);
  const ed = useForumEditor({
    content: null,
    placeholder: "拍品说明：成色、配件、发货方式、售后约定。这份说明会锁进拍卖授权的内容快照。",
    mentionable: [],
  });
  const editor = ed.editor;
  const captcha = usePostCaptcha();

  const create = useMutation({
    mutationFn: (v: AuctionInput) => {
      if (!editor) throw new Error("editor not ready");
      return unwrap(
        api.POST("/api/v1/forums/{slug}/auctions", {
          params: { path: { slug: forumSlug } },
          body: {
            title: v.title.trim(),
            html: editor.getHTML(),
            source: editor.getJSON(),
            captcha: captcha.token,
            ...auctionTerms(v),
          },
        }),
      );
    },
    onSettled: () => captcha.renew(),
    // 草稿已保存：去钱包签拍卖授权，签完钱包回到上线的拍卖帖
    onSuccess: (draft) => location.assign(walletAuctionSignHref(draft.id)),
  });

  const errs = form.formState.errors;
  const v = form.watch();
  const cny = v.currency === "CNY";
  const prices = auctionPrices(v);
  /** 人民币拍卖：按卖家汇率折合的 USDT（与合约换算一致），汇率或金额没填好时不显示 */
  const usdtHint = (amount: bigint | null) => {
    const usdt = cny ? pricesUsdt(prices, amount) : null;
    return usdt === null ? undefined : `折合 ${formatUsdt(usdt)} USDT`;
  };
  const unit = cny ? "人民币" : "USDT";
  const walletMissing = create.error instanceof ApiError && create.error.code === "WALLET_REQUIRED";

  return (
    <form
      className="space-y-4"
      noValidate
      aria-label="发起拍卖"
      onSubmit={form.handleSubmit((values) => {
        if (!editor || editor.isEmpty) return setBodyError("请填写拍品说明");
        setBodyError(null);
        create.mutate(values);
      })}
    >
      <Field label="标题" error={errs.title?.message}>
        <Input maxLength={120} {...form.register("title")} />
      </Field>
      <div>
        <span className="mb-1 block text-sm font-medium">说明</span>
        <EditorField {...ed} invalid={bodyError !== null} />
        {bodyError ? (
          <span role="alert" className="mt-1 block text-sm text-danger">
            {bodyError}
          </span>
        ) : null}
      </div>

      <fieldset className="space-y-2">
        <legend className="text-sm font-medium">计价币种</legend>
        <div className="flex gap-4 text-sm">
          {(["USDT", "CNY"] as const).map((c) => (
            <label key={c} className="flex items-center gap-2">
              <input type="radio" value={c} className="size-4" {...form.register("currency")} />
              {c === "USDT" ? "USDT" : "人民币（按我设定的汇率换算 USDT 结算）"}
            </label>
          ))}
        </div>
      </fieldset>

      <div className="grid gap-4 sm:grid-cols-2">
        {cny ? (
          <Field
            label="我的汇率（1 USDT 合多少元）"
            hint="写进拍卖授权、由你签名，成交时合约按它换算 USDT，平台改不了"
            error={errs.cnyRate?.message}
          >
            <Input inputMode="decimal" placeholder="如 7.2" {...form.register("cnyRate")} />
          </Field>
        ) : null}
        <Field
          label={`起拍价（${unit}）`}
          hint={usdtHint(prices.start)}
          error={errs.start?.message}
        >
          <Input inputMode="decimal" className="amount" {...form.register("start")} />
        </Field>
        <Field
          label={`加价幅度（${unit}）`}
          hint={
            [minIncrementHint(v), usdtHint(prices.increment)].filter(Boolean).join(" · ") ||
            undefined
          }
          error={errs.increment?.message}
        >
          <Input inputMode="decimal" className="amount" {...form.register("increment")} />
        </Field>
        <Field
          label={`保留价（${unit}，选填）`}
          hint={
            usdtHint(v.reserve.trim() ? prices.reserve : null) ??
            "不填 = 有人出价就成交；出价人看不到具体数额"
          }
          error={errs.reserve?.message}
        >
          <Input inputMode="decimal" className="amount" {...form.register("reserve")} />
        </Field>
        <Field label="拍卖时长" error={errs.duration?.message}>
          <select className={selectClass} {...form.register("duration")}>
            {durations.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </Field>
        <Field label="出价资格" error={errs.eligibility?.message}>
          <select className={selectClass} {...form.register("eligibility")}>
            {ELIGIBILITY_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </Field>
        {v.eligibility === "fixed" ? (
          <Field label="余额门槛（USDT）" error={errs.eligibilityAmount?.message}>
            <Input inputMode="decimal" className="amount" {...form.register("eligibilityAmount")} />
          </Field>
        ) : null}
        <Field label="发货期限（天）" error={errs.shipDays?.message}>
          <Input inputMode="numeric" disabled={v.instant} {...form.register("shipDays")} />
        </Field>
        <Field label="买家确认期（天）" error={errs.confirmDays?.message}>
          <Input inputMode="numeric" {...form.register("confirmDays")} />
        </Field>
        <Field label="手续费承担方">
          <select className={selectClass} {...form.register("feePayer")}>
            {FEE_PAYERS.map((f, i) => (
              <option key={f.value} value={String(i)}>
                {f.label}
              </option>
            ))}
          </select>
        </Field>
        <label className="flex items-center gap-2 self-end pb-2 text-sm">
          <input type="checkbox" className="size-4" {...form.register("instant")} />
          即时发货（付款确认后自动发货，如卡密）
        </label>
      </div>
      <p className="text-xs text-fg-muted">
        发布后在钱包里用 Face ID
        签「拍卖授权」（保留价、汇率、成交后的担保条款），签名后拍卖才开始；
        出价在论坛内进行，结束时达到保留价即成交，赢家付款建担保订单。你本人不能出价。
      </p>
      {create.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(create.error)}
          {walletMissing ? (
            <>
              {" "}
              <WalletLink href={`${WALLET_ORIGIN}/setup`} className="underline">
                开通交易
              </WalletLink>
            </>
          ) : null}
        </p>
      ) : null}
      {captcha.widget}
      <div className="flex justify-end">
        <Button
          type="submit"
          disabled={
            create.isPending ||
            create.isSuccess ||
            !editor ||
            ed.upload?.kind === "busy" ||
            captcha.missing
          }
        >
          {create.isPending ? "发布中…" : create.isSuccess ? "正在打开钱包…" : "发布并去钱包签名"}
        </Button>
      </div>
    </form>
  );
}

/** 发起拍卖表单（按需加载，见 AuctionFormLoader）。 */
export default function AuctionForm(props: Props) {
  return (
    <QueryProvider>
      <Form {...props} />
    </QueryProvider>
  );
}
