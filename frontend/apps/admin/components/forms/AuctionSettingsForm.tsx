"use client";

import { errorMessage, type Schemas } from "@akari/api-client";
import { Badge, Button, Card, CardTitle, Field, Input } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { toast } from "@akari/ui/toast";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api, unwrap } from "../../lib/api";
import {
  AUCTION_FIELDS,
  auctionProblems,
  fromDisplay,
  toDisplay,
  type AuctionSettings,
  type NumericKey,
} from "../../lib/auction-settings";
import { keys } from "../../lib/queries";
import { useAdminSession } from "../session/SessionProvider";

type View = Schemas["AuctionSettingsView"];
type Draft = Record<NumericKey, string>;

const asDraft = (s: AuctionSettings): Draft =>
  Object.fromEntries(Object.entries(toDisplay(s)).map(([k, v]) => [k, String(v)])) as Draft;

const parse = (d: Draft): Record<NumericKey, number> =>
  Object.fromEntries(
    Object.entries(d).map(([k, v]) => [k, v.trim() === "" ? Number.NaN : Number(v)]),
  ) as Record<NumericKey, number>;

/** 拍卖开关与参数（只有站长有权限，服务端再校验上下限；每次保存都要 Passkey 再验证并写审计日志）。 */
export default function AuctionSettingsForm({ view }: { view: View }) {
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<Draft>(() => asDraft(view.settings));
  const [errors, setErrors] = useState<Partial<Record<NumericKey, string>>>({});
  const [confirm, setConfirm] = useState(false);
  const saved = asDraft(view.settings);
  const dirty = AUCTION_FIELDS.some(({ key }) => draft[key] !== saved[key]);

  const put = (body: AuctionSettings) =>
    sensitive(() => unwrap(api.PUT("/api/v1/admin/settings/auction", { body })));
  const adopt = (next: View) => {
    queryClient.setQueryData(keys.auctionSettings, next);
    toast.success("已保存，并已推送管理员群");
  };
  const save = useMutation({
    mutationFn: put,
    onSuccess: (next) => {
      adopt(next);
      setDraft(asDraft(next.settings));
      setErrors({});
    },
  });
  // 开关单独保存：用库里现在的参数，只翻转开关，不顺带提交、也不丢掉还没保存的参数修改
  const flip = useMutation({
    mutationFn: put,
    onSuccess: (next) => {
      adopt(next);
      setConfirm(false);
    },
  });

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const body = fromDisplay(parse(draft), view.settings.enabled);
    const problems = auctionProblems(body, view.min, view.max);
    setErrors(problems);
    if (Object.keys(problems).length === 0) save.mutate(body);
  }

  const enabled = view.settings.enabled;

  return (
    <div className="space-y-4">
      <Card className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle>拍卖开关</CardTitle>
          <Badge tone={enabled ? "success" : "neutral"}>{enabled ? "已开启" : "已关闭"}</Badge>
        </div>
        <p className="text-sm text-fg-muted">
          关闭后不能新挂拍卖、不能出价；已结束的拍卖照常结算，赢家照常付款。拍卖帖只能发在允许拍卖的板块（「板块」页设置）。
        </p>
        <Button
          variant={enabled ? "danger" : "primary"}
          disabled={flip.isPending}
          onClick={() => {
            flip.reset();
            setConfirm(true);
          }}
        >
          {enabled ? "关闭拍卖" : "开启拍卖"}
        </Button>
      </Card>

      <Card>
        <form className="space-y-3" noValidate onSubmit={submit} aria-label="拍卖参数">
          <div className="grid gap-3 md:grid-cols-2">
            {AUCTION_FIELDS.map(({ key, label, unit, scale }) => (
              <Field
                key={key}
                label={`${label}（${unit}）`}
                hint={`${view.min[key] / scale}–${view.max[key] / scale} ${unit}，默认 ${view.defaults[key] / scale}`}
                error={errors[key]}
              >
                <Input
                  type="number"
                  inputMode="decimal"
                  step="any"
                  value={draft[key]}
                  aria-invalid={errors[key] !== undefined}
                  onChange={(e) => {
                    const value = e.target.value;
                    setDraft((d) => ({ ...d, [key]: value }));
                    setErrors((errs) => ({ ...errs, [key]: undefined }));
                  }}
                />
              </Field>
            ))}
          </div>
          {save.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(save.error)}
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <Button type="submit" disabled={save.isPending || !dirty}>
              {save.isPending ? "保存中…" : "保存参数"}
            </Button>
            <Button
              variant="secondary"
              onClick={() => {
                setDraft(asDraft(view.defaults));
                setErrors({});
              }}
            >
              填入默认值
            </Button>
          </div>
        </form>
      </Card>

      <Dialog open={confirm} onOpenChange={(o) => (o ? undefined : setConfirm(false))}>
        <DialogContent
          title={enabled ? "关闭拍卖？" : "开启拍卖？"}
          description={
            enabled
              ? "关闭后所有人都不能新挂拍卖、不能出价。进行中的拍卖到期后照常结算。"
              : "开启后卖家可以在允许拍卖的板块发拍卖帖，所有人都可以出价。"
          }
        >
          {flip.isError ? (
            <p role="alert" className="text-sm text-danger">
              {errorMessage(flip.error)}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setConfirm(false)}>
              取消
            </Button>
            <Button
              variant={enabled ? "danger" : "primary"}
              disabled={flip.isPending}
              onClick={() => flip.mutate({ ...view.settings, enabled: !enabled })}
            >
              {flip.isPending ? "提交中…" : enabled ? "确认关闭" : "确认开启"}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
