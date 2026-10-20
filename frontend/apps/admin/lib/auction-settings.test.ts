import { describe, expect, it } from "vitest";
import { auctionProblems, fromDisplay, toDisplay, type AuctionSettings } from "./auction-settings";

const defaults: AuctionSettings = {
  enabled: false,
  min_duration_secs: 86_400,
  max_duration_secs: 604_800,
  snipe_window_secs: 900,
  snipe_extension_secs: 900,
  extension_cap_secs: 86_400,
  pay_window_secs: 172_800,
  runner_up_window_secs: 86_400,
  default_ban_secs: 2_592_000,
  new_account_days: 7,
  new_account_cap_usdt: 100,
};
// 与后端 AuctionSettings::bounds 一致
const min: AuctionSettings = {
  enabled: false,
  min_duration_secs: 3600,
  max_duration_secs: 3600,
  snipe_window_secs: 60,
  snipe_extension_secs: 60,
  extension_cap_secs: 3600,
  pay_window_secs: 3600,
  runner_up_window_secs: 3600,
  default_ban_secs: 86_400,
  new_account_days: 0,
  new_account_cap_usdt: 1,
};
const max: AuctionSettings = {
  enabled: true,
  min_duration_secs: 2_592_000,
  max_duration_secs: 2_592_000,
  snipe_window_secs: 3600,
  snipe_extension_secs: 3600,
  extension_cap_secs: 259_200,
  pay_window_secs: 604_800,
  runner_up_window_secs: 259_200,
  default_ban_secs: 31_536_000,
  new_account_days: 90,
  new_account_cap_usdt: 10_000,
};

describe("auction settings form", () => {
  it("shows hours / minutes / days and converts back losslessly", () => {
    const d = toDisplay(defaults);
    expect(d.min_duration_secs).toBe(24);
    expect(d.snipe_window_secs).toBe(15);
    expect(d.default_ban_secs).toBe(30);
    expect(d.new_account_cap_usdt).toBe(100);
    expect(fromDisplay(d, true)).toEqual({ ...defaults, enabled: true });
  });

  it("keeps fractional server values and rounds input to whole seconds", () => {
    const d = toDisplay({ ...defaults, pay_window_secs: 5400 });
    expect(d.pay_window_secs).toBe(1.5);
    expect(fromDisplay({ ...d, snipe_window_secs: 1.0001 }, false).snipe_window_secs).toBe(60);
  });

  it("accepts the defaults and the bounds themselves", () => {
    expect(auctionProblems(defaults, min, max)).toEqual({});
    expect(auctionProblems({ ...min, enabled: true }, min, max)).toEqual({});
    expect(auctionProblems({ ...max, enabled: false }, min, max)).toEqual({});
  });

  it("flags out-of-range values in display units", () => {
    expect(auctionProblems({ ...defaults, snipe_window_secs: 30 }, min, max)).toEqual({
      snipe_window_secs: "不能小于 1 分钟",
    });
    expect(auctionProblems({ ...defaults, default_ban_secs: 400 * 86_400 }, min, max)).toEqual({
      default_ban_secs: "不能大于 365 天",
    });
    expect(auctionProblems({ ...defaults, new_account_cap_usdt: Number.NaN }, min, max)).toEqual({
      new_account_cap_usdt: "请输入数字",
    });
  });

  it("requires min duration ≤ max duration", () => {
    expect(
      auctionProblems({ ...defaults, min_duration_secs: 7200, max_duration_secs: 3600 }, min, max),
    ).toEqual({ max_duration_secs: "最长时长不能小于最短时长" });
  });
});
