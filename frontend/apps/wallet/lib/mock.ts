// 设计稿模拟数据；接入 forum-api 后删除。金额为 USDT 最小单位字符串。
import { DEFAULT_PARAMS, orderCost, streamNet, streamUnlocked } from "@akari/fees";

export const ESCROW = "0x7a3b91c04e2d5f68a9b1c3d5e7f9a1b3c5d7e9f1";
export const STREAM_ESCROW = "0x5c2e8a1f3b4d6e7a9c0b1d2e3f4a5b6c7d8e9f0a";
export const MY_WALLET = "0x3f9a6c21d84b07e5a1c2d3e4f5a6b7c8d9e0b1c2";

export const wallet = {
  address: MY_WALLET,
  balance: "52340000",
};

export const listing = {
  id: 101,
  title: "JetBrains 全家桶一年授权，即时发货",
  seller: {
    name: "小林",
    level: 5,
    online: true,
    reputation: { successPermille: 986, deals: 214, disputesLost: 1, replyMinutes: 10 },
  },
  price: 45_000_000n,
  feePayer: "seller" as const,
  bond: "45000000",
  confirmDays: 1,
};

export function checkoutCost() {
  const c = orderCost({
    amount: listing.price,
    feePayer: listing.feePayer,
    campaign: false,
    ...DEFAULT_PARAMS,
  });
  return {
    amount: listing.price.toString(),
    buyerFee: (c.fee.buyer + c.gas.buyer).toString(),
    sponsorFee: DEFAULT_PARAMS.sponsorFee.toString(),
    total: (c.buyerPays + DEFAULT_PARAMS.sponsorFee).toString(),
    sellerGets: (listing.price - c.fee.seller - c.gas.seller).toString(),
    disputeFee: c.disputeFee.toString(),
  };
}

const streamAmount = 300_000_000n;
const streamCost = orderCost({
  amount: streamAmount,
  feePayer: "seller",
  campaign: false,
  ...DEFAULT_PARAMS,
});
const net = streamNet(streamCost.fee.seller, streamCost.gas.seller, streamAmount);

export const orders = {
  "1001": {
    id: "1001",
    mode: "once" as const,
    title: listing.title,
    counterpart: listing.seller,
    amount: "45000000",
    state: "delivered" as const,
    hint: "确认期还剩 21 小时，期满自动放款",
    card: "XXXX-7Q2M-JB26-ALLP",
  },
  "1002": {
    id: "1002",
    mode: "stream" as const,
    title: "海外服务器代运维，按天计费",
    counterpart: {
      name: "阿青",
      level: 3,
      online: false,
      reputation: { successPermille: 1000, deals: 3, disputesLost: 0 },
    },
    amount: streamAmount.toString(),
    periods: 30,
    elapsed: 12,
    net: net.toString(),
    unlocked: streamUnlocked(net, 12n, 30n).toString(),
    claimed: streamUnlocked(net, 11n, 30n).toString(),
  },
};

export type OrderId = keyof typeof orders;
export const orderIds = Object.keys(orders) as OrderId[];

export const chat = [
  { id: "1", kind: "system" as const, text: "买家已付款 45.050000 USDT · 已上链", time: "10:02" },
  { id: "2", kind: "system" as const, text: "卡密已自动发放，确认期开始（1 天）", time: "10:02" },
  { id: "3", kind: "them" as const, text: "卡密发了，激活遇到问题随时说～", time: "10:03" },
  {
    id: "4",
    kind: "me" as const,
    text: "提示「许可证已被使用」，能帮看下吗？",
    time: "10:20",
    read: true,
  },
  {
    id: "5",
    kind: "them" as const,
    text: "我换一个给你，或者你加我微信线下转账退你也行",
    time: "10:24",
  },
];
