"use client";

import { ApiError, errorMessage, type Schemas } from "@akari/api-client";
import { Button, Field, Input } from "@akari/ui";
import { standardSchemaResolver } from "@hookform/resolvers/standard-schema";
import { useMutation } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { useForm } from "react-hook-form";
import { api, unwrap } from "../../lib/browser-api";
import { FEE_PAYERS } from "../../lib/fees";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { EditorField, useForumEditor } from "../composer/Editor";
import { QueryProvider } from "../QueryProvider";
import { listingSchema, listingTerms, type ListingInput } from "./listing-schema";

const DAY = 86_400;
const selectClass = "h-10 w-full rounded-md border border-border bg-surface px-3 text-fg";

interface Props {
  forumSlug: string;
  params: Schemas["TradeParams"];
}

function Form({ forumSlug, params }: Props) {
  const router = useRouter();
  const schema = useMemo(() => listingSchema(params), [params]);
  const d = params.defaults;
  const form = useForm<ListingInput>({
    resolver: standardSchemaResolver(schema),
    defaultValues: {
      title: "",
      price: "",
      instant: false,
      shipDays: String(Math.round(d.ship_window / DAY)),
      confirmDays: String(Math.round(d.confirm_window / DAY)),
      feePayer: String(d.fee_payer) as ListingInput["feePayer"],
      bond: "",
      cnyRate: "",
    },
  });
  const [bodyError, setBodyError] = useState<string | null>(null);
  const { editor, menu } = useForumEditor({
    content: null,
    placeholder: "商品或服务说明：交付内容、发货方式、售后约定。这份说明会锁进报价的内容快照。",
    mentionable: [],
  });

  const create = useMutation({
    mutationFn: (v: ListingInput) => {
      if (!editor) throw new Error("editor not ready");
      return unwrap(
        api.POST("/api/v1/forums/{slug}/listings", {
          params: { path: { slug: forumSlug } },
          body: {
            title: v.title.trim(),
            html: editor.getHTML(),
            source: editor.getJSON(),
            ...listingTerms(v),
          },
        }),
      );
    },
    onSuccess: (created) => router.push(`/trade/${created.listing_id}`),
  });

  const errs = form.formState.errors;
  const instant = form.watch("instant");
  const walletMissing = create.error instanceof ApiError && create.error.code === "WALLET_REQUIRED";

  return (
    <form
      className="space-y-4"
      noValidate
      aria-label="发起交易帖"
      onSubmit={form.handleSubmit((v) => {
        if (!editor || editor.isEmpty) return setBodyError("请填写商品说明");
        setBodyError(null);
        create.mutate(v);
      })}
    >
      <Field label="标题" error={errs.title?.message}>
        <Input maxLength={120} {...form.register("title")} />
      </Field>
      <div>
        <span className="mb-1 block text-sm font-medium">说明</span>
        <EditorField editor={editor} menu={menu} invalid={bodyError !== null} />
        {bodyError ? (
          <span role="alert" className="mt-1 block text-sm text-danger">
            {bodyError}
          </span>
        ) : null}
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="价格（USDT）" error={errs.price?.message}>
          <Input inputMode="decimal" placeholder="如 45 或 12.5" {...form.register("price")} />
        </Field>
        <Field label="卖家保证金（USDT，可选）" error={errs.bond?.message}>
          <Input inputMode="decimal" placeholder="0" {...form.register("bond")} />
        </Field>
        <Field label="人民币参考汇率（选填，1 USDT 合多少元）" error={errs.cnyRate?.message}>
          <Input inputMode="decimal" placeholder="如 7.2" {...form.register("cnyRate")} />
        </Field>
        <Field label="发货期限（天）" error={errs.shipDays?.message}>
          <Input inputMode="numeric" disabled={instant} {...form.register("shipDays")} />
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
        发帖后在钱包里签名报价，买家才能担保购买；成交以签名报价为准。填了参考汇率时页面另显示人民币参考价，
        只是参考，付款仍按 USDT。图片上传即将开放。
      </p>
      {create.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(create.error)}
          {walletMissing ? (
            <>
              {" "}
              <a href={`${WALLET_ORIGIN}/setup`} className="underline">
                开通交易
              </a>
            </>
          ) : null}
        </p>
      ) : null}
      <div className="flex justify-end">
        <Button type="submit" disabled={create.isPending || !editor}>
          {create.isPending ? "发布中…" : "发布交易帖"}
        </Button>
      </div>
    </form>
  );
}

/** 发起交易帖表单（按需加载，见 ListingFormLoader）。 */
export default function ListingForm(props: Props) {
  return (
    <QueryProvider>
      <Form {...props} />
    </QueryProvider>
  );
}
