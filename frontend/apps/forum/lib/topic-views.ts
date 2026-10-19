// 接口数据 → 论坛共享组件的显示模型（主题列表行、标题区、名片）。纯函数，单元测试直接测。
import type { Schemas } from "@akari/api-client";
import type { PersonCard } from "../components/forum/UserPopover";
import type { AuctionFacts } from "../components/forum/AuctionLine";
import type { TopicHeaderView } from "../components/forum/TopicHeader";
import type { TopicKind, TopicRowView } from "../components/forum/TopicTable";
import type { TradeFacts } from "../components/forum/TradeLine";
import {
  formatActivity,
  profileHref,
  tradeReputation,
  userName,
  type UserCardView,
} from "./display";
import { auctionCurrency, secondsUntil } from "./auction";
import { topicHref } from "./links";
import { boardRef } from "./nav";

/** TopicSummary.kind：0 普通、1 交易、2 争议、3 拍卖。 */
const TOPIC_KIND_TRADE = 1;
const TOPIC_KIND_AUCTION = 3;

export function isAuctionTopic(t: Pick<Schemas["TopicSummary"], "kind" | "auction">): boolean {
  return t.kind === TOPIC_KIND_AUCTION || t.auction != null;
}

type Lookup = (id: number) => UserCardView;

export function topicKind(t: Pick<Schemas["TopicSummary"], "kind" | "auction">): TopicKind {
  if (t.kind === TOPIC_KIND_TRADE) return "trade";
  return isAuctionTopic(t) ? "auction" : "normal";
}

/** 有过交易记录才在名片上显示信誉（否则人人都是「新卖家 · 已成交 0 笔」）。 */
function hasTraded(r: Schemas["ReputationView"]): boolean {
  return r.success + r.failed + r.settled > 0;
}

export function personCard(u: UserCardView): PersonCard {
  return {
    id: u.id,
    name: userName(u),
    href: profileHref(u),
    level: u.level,
    avatarUrl: u.avatar_url,
    reputation: hasTraded(u.reputation) ? tradeReputation(u.reputation) : undefined,
  };
}

/** 交易帖进 /trade/{listing}（价格面板）；其余进主题页。 */
export function topicLink(t: Schemas["TopicSummary"]): string {
  return topicKind(t) === "trade" && t.trade ? `/trade/${t.trade.listing_id}` : topicHref(t.id);
}

export function topicRowView(
  t: Schemas["TopicSummary"],
  user: Lookup,
  opts: { forum?: Schemas["ForumView"] | undefined; now?: Date } = {},
): TopicRowView {
  return {
    id: t.id,
    title: t.title,
    href: topicLink(t),
    kind: topicKind(t),
    board: opts.forum ? boardRef(opts.forum) : undefined,
    author: personCard(user(t.author_id)),
    participants: t.participants.map((id) => {
      const u = user(id);
      return { name: userName(u), avatarUrl: u.avatar_url };
    }),
    replies: t.replies,
    views: t.views,
    activityIso: t.last_post_at,
    activity: formatActivity(t.last_post_at, opts.now),
    pinned: t.flags.pinned,
    featured: t.flags.featured,
    locked: t.flags.locked,
    hidden: t.flags.hidden,
  };
}

/** 列表行里交易帖的摘要（卖家就是楼主，成功率取楼主名片上的信誉）；不是交易帖时返回 null。 */
export function tradeFacts(t: Schemas["TopicSummary"], user: Lookup): TradeFacts | null {
  const trade = t.trade;
  if (topicKind(t) !== "trade" || !trade) return null;
  const seller = user(t.author_id).reputation;
  return {
    price: trade.price,
    cnyPerUsdt: trade.cny_per_usdt,
    instant: trade.instant,
    stock: trade.stock,
    seller: hasTraded(seller) ? tradeReputation(seller) : undefined,
  };
}

/** 列表行里拍卖帖的摘要；剩余时间按渲染时刻算（服务端渲染用服务器时钟）。不是拍卖帖或币种不认识时返回 null。 */
export function auctionFacts(t: Schemas["TopicSummary"], now = new Date()): AuctionFacts | null {
  const a = t.auction;
  const currency = a ? auctionCurrency(a.currency) : null;
  if (!a || !currency) return null;
  return {
    currency,
    current: a.current,
    bids: a.bids,
    reserveMet: a.reserve_met,
    secondsLeft: secondsUntil(a.ends_at, now.getTime(), 0),
  };
}

export function topicHeaderView(
  t: Schemas["TopicSummary"],
  forum: Schemas["ForumView"],
): TopicHeaderView {
  return {
    title: t.title,
    kind: topicKind(t),
    board: boardRef(forum),
    pinned: t.flags.pinned,
    locked: t.flags.locked,
    featured: t.flags.featured,
    hidden: t.flags.hidden,
    replies: t.replies,
    views: t.views,
  };
}
