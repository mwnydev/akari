import { DEFAULT_PARAMS } from "@akari/fees";

// 代付费档位（纯数据，可以进首屏 JS：确认页要先显示费用）。代付客户端本身在 ./sponsor.ts，只进钱包 SDK 分包。

/**
 * 需要平台代付的操作。费用档位（ADR 0004）：付款前的操作（下单、提回交易所 = USDT `transfer`）收固定代付费，
 * 付款后的操作免费（结算时扣固定 gas 费；分期 `claim` 另从当次领取额里扣领取 gas 费）。
 */
export type SponsoredAction =
  "createAndFund" | "transfer" | "markDelivered" | "release" | "start" | "claim";

const FEE_ACTIONS: ReadonlySet<SponsoredAction> = new Set(["createAndFund", "transfer"]);

/**
 * 确认页展示的代付费，与 forum-sponsor 的规则一致：付款前的操作收费，付款后的操作免费。
 * 站长调价后这里与代付服务不一致时，`assertSponsorFee` 会拒签，不会多扣用户的钱。
 */
export function sponsorFeeFor(action: SponsoredAction): bigint {
  return FEE_ACTIONS.has(action) ? DEFAULT_PARAMS.sponsorFee : 0n;
}
