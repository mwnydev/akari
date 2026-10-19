// 设计稿用的模拟数据；接入 API 后删除。金额为 USDT 最小单位字符串。
import type { TradeReputation } from "@akari/ui";

export interface MockUser {
  name: string;
  level: number;
  levelName: string;
  online: boolean;
  lastSeen?: string;
  reputation?: TradeReputation;
  joined: string;
  signature?: string;
}

export const users: Record<string, MockUser> = {
  小林: {
    name: "小林",
    level: 5,
    levelName: "老会员",
    online: true,
    reputation: { successPermille: 986, deals: 214, disputesLost: 1, replyMinutes: 10 },
    joined: "2026-03-02",
    signature: "专营各类正版激活码，当天发货。",
  },
  阿青: {
    name: "阿青",
    level: 3,
    levelName: "活跃",
    online: false,
    lastSeen: "5 分钟前",
    reputation: { successPermille: 1000, deals: 3, disputesLost: 0 },
    joined: "2026-09-12",
  },
  老周: {
    name: "老周",
    level: 6,
    levelName: "元老",
    online: false,
    lastSeen: "今天",
    joined: "2025-12-30",
  },
  momo: { name: "momo", level: 1, levelName: "新手", online: true, joined: "2026-10-18" },
};

export function user(name: string): MockUser {
  const u = users[name];
  if (!u) throw new Error(`mock user ${name} missing`);
  return u;
}

export interface Board {
  slug: string;
  name: string;
  description: string;
  topics: number;
  today: number;
  trade?: boolean;
}

export const boards: Board[] = [
  {
    slug: "chat",
    name: "综合讨论",
    description: "闲聊、站务、新人报到",
    topics: 182_340,
    today: 412,
  },
  {
    slug: "tech",
    name: "技术交流",
    description: "编程、服务器、区块链开发",
    topics: 64_201,
    today: 97,
  },
  {
    slug: "digital",
    name: "数字商品交易",
    description: "激活码、会员、软件授权；担保交易",
    topics: 23_118,
    today: 156,
    trade: true,
  },
  {
    slug: "service",
    name: "服务交易",
    description: "代运营、租用、订阅；支持分期担保",
    topics: 9_870,
    today: 41,
    trade: true,
  },
  {
    slug: "help",
    name: "帮助与反馈",
    description: "充值网络、手续费、争议规则",
    topics: 3_204,
    today: 12,
  },
];

export interface Topic {
  id: number;
  board: string;
  title: string;
  author: string;
  replies: number;
  views: number;
  lastPoster: string;
  lastAt: string;
  pinned?: boolean;
  digest?: boolean;
  locked?: boolean;
  trade?: { price: string; mode: "once" | "stream" };
}

export const topics: Topic[] = [
  {
    id: 1,
    board: "help",
    title: "【必读】充值时务必选择 Arbitrum One 网络（图文）",
    author: "老周",
    replies: 88,
    views: 40_211,
    lastPoster: "momo",
    lastAt: "10 分钟前",
    pinned: true,
    locked: true,
  },
  {
    id: 2,
    board: "chat",
    title: "担保交易的手续费和 gas 到底怎么算？一张表说清楚",
    author: "老周",
    replies: 132,
    views: 18_920,
    lastPoster: "阿青",
    lastAt: "23 分钟前",
    digest: true,
  },
  {
    id: 101,
    board: "digital",
    title: "JetBrains 全家桶一年授权，即时发货",
    author: "小林",
    replies: 46,
    views: 3_120,
    lastPoster: "momo",
    lastAt: "2 分钟前",
    trade: { price: "45000000", mode: "once" },
  },
  {
    id: 102,
    board: "service",
    title: "海外服务器代运维，按天计费，支持分期担保",
    author: "阿青",
    replies: 12,
    views: 860,
    lastPoster: "小林",
    lastAt: "1 小时前",
    trade: { price: "300000000", mode: "stream" },
  },
  {
    id: 3,
    board: "tech",
    title: "用 Rust 写 ERC-4337 私有代发器的一些坑",
    author: "老周",
    replies: 57,
    views: 6_402,
    lastPoster: "小林",
    lastAt: "3 小时前",
    digest: true,
  },
  {
    id: 4,
    board: "chat",
    title: "新人报到，从交易所提 USDT 过来要多久？",
    author: "momo",
    replies: 9,
    views: 210,
    lastPoster: "阿青",
    lastAt: "5 小时前",
  },
];

export interface Floor {
  floor: number;
  author: string;
  html: string;
  at: string;
  edited?: boolean;
  quote?: { floor: number; author: string; text: string };
  deleted?: boolean;
}

export const floors: Floor[] = [
  {
    floor: 1,
    author: "老周",
    at: "2026-10-12 21:04",
    edited: true,
    html: "<p>很多新人问到手续费和 gas，统一说明一下：</p><ul><li>手续费默认 1%，最低 1 USDT，由买卖双方协商谁承担；</li><li>gas 全部由平台代付，钱包里<strong>不需要 ETH</strong>；</li><li>付款、提回交易所时收 0.05 USDT 代付费，放款时扣 0.1 USDT 固定 gas 费。</li></ul><p>举例：100 USDT 的订单、卖家承担手续费，买家付 100.05 USDT，卖家到手 98.9 USDT。</p>",
  },
  {
    floor: 2,
    author: "阿青",
    at: "2026-10-12 21:30",
    html: "<p>清楚了，比 TRC20 转来转去便宜多了。</p>",
  },
  {
    floor: 3,
    author: "momo",
    at: "2026-10-13 08:12",
    quote: { floor: 1, author: "老周", text: "钱包里不需要 ETH" },
    html: "<p>那我从币安提币的时候选哪个网络？</p>",
  },
  { floor: 4, author: "小林", at: "2026-10-13 08:20", html: "", deleted: true },
  {
    floor: 5,
    author: "老周",
    at: "2026-10-13 09:01",
    html: "<p>选 <strong>Arbitrum One</strong>，置顶帖有图文教程。选错 TRC20 是找不回来的。</p>",
  },
];

export const trade = {
  id: 101,
  title: "JetBrains 全家桶一年授权，即时发货",
  seller: "小林",
  price: "45000000",
  mode: "once" as const,
  instant: true,
  confirmDays: 1,
  shipDays: 0,
  feePayer: "seller" as const,
  bond: "45000000",
  stock: 12,
  html: "<p>正版 All Products Pack 个人授权，一年期。</p><ul><li>付款上链后自动发卡密，无需等待；</li><li>激活失败在确认期内发起争议，保证金赔付；</li><li>可开具购买记录截图。</li></ul>",
  snapshotHash: "0x9c1f…a2e4",
};
