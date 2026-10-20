import type { Schemas } from "@akari/api-client";

// 拍卖设置表单：服务端按秒存，后台按小时 / 分钟 / 天显示与输入（纯函数，便于测试）。

export type AuctionSettings = Schemas["AuctionSettings"];
export type NumericKey = Exclude<keyof AuctionSettings, "enabled">;

export interface AuctionField {
  key: NumericKey;
  label: string;
  unit: "小时" | "分钟" | "天" | "USDT";
  /** 一个显示单位是多少个服务端单位（秒；天数与 USDT 本身就是 1）。 */
  scale: number;
}

const HOUR = 3600;

export const AUCTION_FIELDS: readonly AuctionField[] = [
  { key: "min_duration_secs", label: "卖家可选的最短拍卖时长", unit: "小时", scale: HOUR },
  { key: "max_duration_secs", label: "卖家可选的最长拍卖时长", unit: "小时", scale: HOUR },
  {
    key: "snipe_window_secs",
    label: "防狙击窗口：结束前多久的出价触发顺延",
    unit: "分钟",
    scale: 60,
  },
  { key: "snipe_extension_secs", label: "每次顺延", unit: "分钟", scale: 60 },
  { key: "extension_cap_secs", label: "累计顺延上限", unit: "小时", scale: HOUR },
  { key: "pay_window_secs", label: "赢家付款期", unit: "小时", scale: HOUR },
  { key: "runner_up_window_secs", label: "次高出价者递补付款期", unit: "小时", scale: HOUR },
  { key: "default_ban_secs", label: "违约默认禁拍", unit: "天", scale: 86_400 },
  { key: "new_account_days", label: "新账号：注册不满", unit: "天", scale: 1 },
  { key: "new_account_cap_usdt", label: "新账号单场出价上限", unit: "USDT", scale: 1 },
];

/** 表单里显示的值（服务端值不是整单位时保留小数，不悄悄改掉）。 */
export function toDisplay(settings: AuctionSettings): Record<NumericKey, number> {
  return Object.fromEntries(
    AUCTION_FIELDS.map(({ key, scale }) => [key, settings[key] / scale]),
  ) as Record<NumericKey, number>;
}

/** 表单值换回服务端单位（取整到秒）；开关原样保留。 */
export function fromDisplay(values: Record<NumericKey, number>, enabled: boolean): AuctionSettings {
  const out = { enabled } as AuctionSettings;
  for (const { key, scale } of AUCTION_FIELDS) out[key] = Math.round(values[key] * scale);
  return out;
}

/** 与服务端 `AuctionSettings::is_valid` 相同的规则；返回每项的错误文字（全部合格返回空对象）。 */
export function auctionProblems(
  s: AuctionSettings,
  min: AuctionSettings,
  max: AuctionSettings,
): Partial<Record<NumericKey, string>> {
  const out: Partial<Record<NumericKey, string>> = {};
  for (const { key, unit, scale } of AUCTION_FIELDS) {
    const v = s[key];
    if (!Number.isFinite(v)) out[key] = "请输入数字";
    else if (v < min[key]) out[key] = `不能小于 ${min[key] / scale} ${unit}`;
    else if (v > max[key]) out[key] = `不能大于 ${max[key] / scale} ${unit}`;
  }
  if (
    !out.max_duration_secs &&
    !out.min_duration_secs &&
    s.min_duration_secs > s.max_duration_secs
  ) {
    out.max_duration_secs = "最长时长不能小于最短时长";
  }
  return out;
}
