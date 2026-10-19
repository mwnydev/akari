// 设计稿 v2 的模拟数据：只供 /design/v2/* 展示，不调接口。字段按页面需要取舍，不代表最终接口形状。
import type { TradeReputation } from "@akari/ui";

export interface MockUser {
  id: number;
  name: string;
  level: number;
  levelName?: string;
  online: boolean;
  /** 不在线时的最近活跃 */
  lastSeen?: string;
  role?: "版主" | "管理员";
  joined: string;
  posts: number;
  likes: number;
  /** 只统计链上已最终确认的订单；没有交易记录时不传 */
  reputation?: TradeReputation;
}

const users: MockUser[] = [
  {
    id: 1,
    name: "小林",
    level: 6,
    levelName: "老会员",
    online: true,
    role: "版主",
    joined: "2026-03",
    posts: 1820,
    likes: 5410,
    reputation: { successPermille: 986, deals: 214, disputesLost: 1, replyMinutes: 10 },
  },
  {
    id: 2,
    name: "阿青",
    level: 3,
    online: false,
    lastSeen: "5 分钟前",
    joined: "2026-08",
    posts: 96,
    likes: 140,
    reputation: { successPermille: 1000, deals: 3, disputesLost: 0 },
  },
  { id: 3, name: "momo", level: 4, online: true, joined: "2026-05", posts: 402, likes: 980 },
  {
    id: 4,
    name: "王五",
    level: 2,
    online: false,
    lastSeen: "1 小时前",
    joined: "2026-09",
    posts: 31,
    likes: 22,
  },
  {
    id: 5,
    name: "张三",
    level: 5,
    online: true,
    joined: "2026-04",
    posts: 760,
    likes: 2100,
    reputation: { successPermille: 972, deals: 58, disputesLost: 0, replyMinutes: 25 },
  },
  {
    id: 6,
    name: "李四",
    level: 3,
    online: false,
    lastSeen: "昨天",
    joined: "2026-07",
    posts: 120,
    likes: 310,
  },
  {
    id: 7,
    name: "星野",
    level: 7,
    levelName: "元老",
    online: true,
    role: "管理员",
    joined: "2026-01",
    posts: 3300,
    likes: 12800,
  },
  {
    id: 8,
    name: "橘子汽水",
    level: 4,
    online: true,
    joined: "2026-06",
    posts: 288,
    likes: 640,
    reputation: { successPermille: 993, deals: 141, disputesLost: 0, replyMinutes: 5 },
  },
  {
    id: 9,
    name: "老周",
    level: 5,
    online: false,
    lastSeen: "3 小时前",
    joined: "2026-02",
    posts: 950,
    likes: 2600,
    reputation: { successPermille: 1000, deals: 37, disputesLost: 0, replyMinutes: 30 },
  },
  { id: 10, name: "Kiki", level: 2, online: true, joined: "2026-10", posts: 12, likes: 9 },
  {
    id: 11,
    name: "北冥",
    level: 4,
    online: false,
    lastSeen: "20 分钟前",
    joined: "2026-05",
    posts: 330,
    likes: 700,
  },
  { id: 12, name: "夜航船", level: 3, online: true, joined: "2026-08", posts: 75, likes: 160 },
];

/** 设计稿里的当前访问者：小林（交易板块版主），所以能看到管理菜单 */
export const VIEWER_ID = 1;

export function user(id: number): MockUser {
  const u = users.find((x) => x.id === id);
  if (!u) throw new Error(`mock user ${id} missing`);
  return u;
}

/** 板块色块，对应 globals.css 里的 --color-board-* 令牌。 */
export type BoardColor = "red" | "blue" | "green" | "orange" | "purple" | "teal";

export interface Board {
  slug: string;
  name: string;
  color: BoardColor;
  description: string;
  topics: number;
  today: number;
}

export const boards: Board[] = [
  {
    slug: "notice",
    name: "公告",
    color: "red",
    description: "站务公告、规则更新与手续费活动",
    topics: 18,
    today: 0,
  },
  {
    slug: "general",
    name: "综合",
    color: "blue",
    description: "闲聊、经验分享与站内讨论",
    topics: 4210,
    today: 86,
  },
  {
    slug: "trade",
    name: "交易",
    color: "green",
    description: "担保交易帖：卖家签名报价，资金托管在合约里",
    topics: 1893,
    today: 41,
  },
  {
    slug: "auction",
    name: "拍卖",
    color: "orange",
    description: "论坛内出价、不花 gas；成交后与卖家建担保订单",
    topics: 236,
    today: 7,
  },
  {
    slug: "tech",
    name: "技术",
    color: "purple",
    description: "钱包、链上、开发与工具",
    topics: 1302,
    today: 19,
  },
  {
    slug: "help",
    name: "帮助",
    color: "teal",
    description: "充值网络、交易流程、争议规则答疑",
    topics: 655,
    today: 12,
  },
];

export function board(slug: string): Board {
  const b = boards.find((x) => x.slug === slug);
  if (!b) throw new Error(`mock board ${slug} missing`);
  return b;
}

export interface TradeFacts {
  /** USDT 最小单位 */
  price: string;
  /** 卖家参考汇率（4 位小数定点数），不填只显示 USDT */
  cnyPerUsdt?: number;
  instant: boolean;
  stock?: number;
  /** 卖家信誉，与用户名片同一份 */
  seller: TradeReputation;
}

export interface AuctionFacts {
  currency: "USDT" | "CNY";
  /** 当前价：USDT 时为最小单位，CNY 时为分 */
  current: string;
  /** 卖家签名汇率，仅 CNY */
  cnyPerUsdt?: number;
  bids: number;
  secondsLeft: number;
  reserveMet: boolean;
}

export interface TopicSummary {
  id: number;
  title: string;
  board: string;
  authorId: number;
  /** 最近发言的人，第一位是作者 */
  participants: number[];
  replies: number;
  views: number;
  /** 最后活动 */
  activity: string;
  activityIso: string;
  pinned?: boolean;
  featured?: boolean;
  locked?: boolean;
  /** 未读回复数（登录用户） */
  unread?: number;
  href: string;
  trade?: TradeFacts;
  auction?: AuctionFacts;
}

const seller1 = user(8).reputation as TradeReputation;
const seller2 = user(5).reputation as TradeReputation;
const seller3 = user(2).reputation as TradeReputation;

export const topics: TopicSummary[] = [
  {
    id: 101,
    title: "【公告】10 月 25 日起担保手续费限时 0 费率（仅担保交易，含 gas 补贴前 1000 单）",
    board: "notice",
    authorId: 7,
    participants: [7, 1],
    replies: 64,
    views: 9820,
    activity: "2 小时",
    activityIso: "2026-10-20T06:10:00Z",
    pinned: true,
    href: "/design/v2/topic",
  },
  {
    id: 102,
    title: "Steam 钱包充值码 $50 · 自动发卡 · 现货",
    board: "trade",
    authorId: 8,
    participants: [8, 3, 10, 5, 12, 6],
    replies: 37,
    views: 2140,
    activity: "3 分钟",
    activityIso: "2026-10-20T08:07:00Z",
    unread: 4,
    href: "/design/v2/trade",
    trade: { price: "45000000", cnyPerUsdt: 72_000, instant: true, stock: 37, seller: seller1 },
  },
  {
    id: 103,
    title: "绝版 1/12 机甲手办 · 未拆封带原盒 · 起拍 ¥500",
    board: "auction",
    authorId: 9,
    participants: [9, 11, 3, 5, 12, 4, 6],
    replies: 52,
    views: 3870,
    activity: "1 分钟",
    activityIso: "2026-10-20T08:09:00Z",
    featured: true,
    href: "/design/v2/auction",
    auction: {
      currency: "CNY",
      current: "128000",
      cnyPerUsdt: 72_000,
      bids: 23,
      secondsLeft: 2 * 3600 + 14 * 60 + 5,
      reserveMet: true,
    },
  },
  {
    id: 104,
    title: "新人求助：从交易所提 USDT 到论坛钱包，选哪个网络？",
    board: "help",
    authorId: 10,
    participants: [10, 7, 1, 3, 9],
    replies: 12,
    views: 431,
    activity: "8 分钟",
    activityIso: "2026-10-20T08:02:00Z",
    href: "/design/v2/topic",
  },
  {
    id: 105,
    title: "代写 Rust 后端小工具 · 按需求报价 · 人工交付",
    board: "trade",
    authorId: 5,
    participants: [5, 11, 6],
    replies: 9,
    views: 512,
    activity: "25 分钟",
    activityIso: "2026-10-20T07:45:00Z",
    href: "/design/v2/trade",
    trade: { price: "200000000", instant: false, seller: seller2 },
  },
  {
    id: 106,
    title: "2026 年还在用的机械键盘，晒一下你的桌面",
    board: "general",
    authorId: 3,
    participants: [3, 6, 4, 12, 9, 11, 5, 2],
    replies: 188,
    views: 7650,
    activity: "12 分钟",
    activityIso: "2026-10-20T07:58:00Z",
    featured: true,
    href: "/design/v2/topic",
  },
  {
    id: 107,
    title: "限量黑胶唱片 · 签名版 · 无保留价",
    board: "auction",
    authorId: 9,
    participants: [9, 3, 12],
    replies: 6,
    views: 344,
    activity: "40 分钟",
    activityIso: "2026-10-20T07:30:00Z",
    href: "/design/v2/auction",
    auction: {
      currency: "USDT",
      current: "36000000",
      bids: 4,
      secondsLeft: 3 * 86_400 + 5 * 3600,
      reserveMet: false,
    },
  },
  {
    id: 108,
    title: "Passkey 换手机后怎么迁移？实测三种方式",
    board: "tech",
    authorId: 11,
    participants: [11, 7, 5, 10],
    replies: 27,
    views: 1290,
    activity: "1 小时",
    activityIso: "2026-10-20T07:10:00Z",
    href: "/design/v2/topic",
  },
  {
    id: 109,
    title: "二手 iPad mini 7 · 256G · 自用无拆修",
    board: "trade",
    authorId: 2,
    participants: [2, 12],
    replies: 3,
    views: 160,
    activity: "2 小时",
    activityIso: "2026-10-20T06:05:00Z",
    href: "/design/v2/trade",
    trade: { price: "310000000", cnyPerUsdt: 71_500, instant: false, stock: 1, seller: seller3 },
  },
  {
    id: 110,
    title: "最近仲裁案例脱敏汇总（10 月第 3 周）",
    board: "general",
    authorId: 7,
    participants: [7, 1, 9],
    replies: 15,
    views: 2033,
    activity: "5 小时",
    activityIso: "2026-10-20T03:10:00Z",
    locked: true,
    href: "/design/v2/topic",
  },
  {
    id: 111,
    title: "复古游戏卡带合集 · 12 盘 · 即将结束",
    board: "auction",
    authorId: 5,
    participants: [5, 4, 6, 11],
    replies: 21,
    views: 980,
    activity: "6 分钟",
    activityIso: "2026-10-20T08:04:00Z",
    href: "/design/v2/auction",
    auction: {
      currency: "USDT",
      current: "88500000",
      bids: 17,
      secondsLeft: 9 * 60 + 41,
      reserveMet: true,
    },
  },
  {
    id: 112,
    title: "周末夜聊：你做过最值的一笔担保交易",
    board: "general",
    authorId: 12,
    participants: [12, 3, 8, 2, 10, 5],
    replies: 73,
    views: 1880,
    activity: "30 分钟",
    activityIso: "2026-10-20T07:40:00Z",
    href: "/design/v2/topic",
  },
];

/** 热门：按浏览量 */
export const hotTopics = [...topics].filter((t) => !t.pinned).sort((a, b) => b.views - a.views);

export interface FloorComment {
  id: number;
  authorId: number;
  /** 回复楼中某人的评论时，被回复者 */
  replyTo?: number;
  text: string;
}

export interface FloorData {
  floor: number;
  authorId: number;
  time: string;
  timeIso: string;
  edited?: boolean;
  body: string[];
  quote?: { floor: number; authorId: number; text: string };
  /** 楼层里嵌了卖家签名的报价卡片 */
  hasOffer?: boolean;
  /** 点赞的人（按时间倒序取前几位），总数见 likeCount */
  likers: number[];
  likeCount: number;
  comments: FloorComment[];
}

const longThread: FloorComment[] = [
  { id: 1, authorId: 4, text: "选 Arbitrum One，别选错，错了要走工单找回。" },
  {
    id: 2,
    authorId: 5,
    replyTo: 4,
    text: "补充：有些交易所写的是「ARB」或「Arbitrum」，都是同一个。",
  },
  { id: 3, authorId: 10, text: "谢谢，已经到账了！" },
  { id: 4, authorId: 3, replyTo: 10, text: "到账后记得先小额试一笔担保，熟悉下流程。" },
  { id: 5, authorId: 9, text: "手续费比我想的低，0.1 USDT 左右。" },
  { id: 6, authorId: 6, replyTo: 9, text: "高峰期会贵一点，不过也就几毛。" },
  { id: 7, authorId: 11, text: "收藏了，回头写进新人帖。" },
  { id: 8, authorId: 1, replyTo: 11, text: "已经加进帮助中心了，置顶帖里有链接。" },
];

export const topicFloors: FloorData[] = [
  {
    floor: 1,
    authorId: 10,
    time: "今天 15:12",
    timeIso: "2026-10-20T07:12:00Z",
    body: [
      "刚注册，想从交易所提 USDT 到论坛钱包，提币页面有好几个网络可选：TRC20、ERC20、Arbitrum One、BSC……",
      "看帮助说只支持一条链，怕选错。有没有老哥说一下具体怎么选？截图里那个「最低提币数量」又是什么意思？",
    ],
    likers: [7, 1, 3],
    likeCount: 6,
    comments: [
      { id: 1, authorId: 3, text: "先看帮助中心「充值时选 Arbitrum One 网络」那篇，有图。" },
    ],
  },
  {
    floor: 2,
    authorId: 7,
    time: "今天 15:20",
    timeIso: "2026-10-20T07:20:00Z",
    body: [
      "只选 Arbitrum One。充值地址页面顶上一直显示网络标识，和交易所提币页面的网络名称对上再提。",
      "最低提币数量是交易所自己的限制，跟论坛无关；第一次建议先提一小笔试试。",
    ],
    likers: [10, 1, 3, 5, 9, 11, 6, 4],
    likeCount: 12,
    comments: longThread,
  },
  {
    floor: 3,
    authorId: 1,
    time: "今天 15:31",
    timeIso: "2026-10-20T07:31:00Z",
    body: ["补充一点：钱包永远不需要 ETH 做 gas，平台代付，付款时会多扣 0.05 USDT 代付费。"],
    quote: { floor: 2, authorId: 7, text: "只选 Arbitrum One。充值地址页面顶上一直显示网络标识……" },
    likers: [10, 5],
    likeCount: 2,
    comments: [],
  },
  {
    floor: 4,
    authorId: 9,
    time: "今天 15:44",
    timeIso: "2026-10-20T07:44:00Z",
    body: ["要是已经选错网络提过来了怎么办？之前在别的地方踩过坑。"],
    likers: [],
    likeCount: 0,
    comments: [
      {
        id: 1,
        authorId: 7,
        replyTo: 9,
        text: "走客服工单「充错网络找回」，同地址的 EVM 链大多能找回。",
      },
      { id: 2, authorId: 9, replyTo: 7, text: "好的，了解。" },
    ],
  },
  {
    floor: 5,
    authorId: 3,
    time: "今天 15:58",
    timeIso: "2026-10-20T07:58:00Z",
    body: ["我一般先提 5 USDT 试水，到账了再提剩下的，安心。"],
    likers: [10, 12, 6],
    likeCount: 3,
    comments: [],
  },
  {
    floor: 6,
    authorId: 5,
    time: "今天 16:02",
    timeIso: "2026-10-20T08:02:00Z",
    edited: true,
    body: [
      "顺便推荐一下：充值完先去交易板块找个小额自动发卡的帖子买一单，整个担保流程走一遍就懂了。",
    ],
    likers: [8],
    likeCount: 1,
    comments: [{ id: 1, authorId: 8, text: "欢迎来我这儿试单" }],
  },
];

export const tradeFloors: FloorData[] = [
  {
    floor: 1,
    authorId: 8,
    time: "10-12 14:00",
    timeIso: "2026-10-12T06:00:00Z",
    edited: true,
    body: [
      "美区 Steam 钱包充值码，面值 $50，官方渠道采购，付款后系统自动发卡（卡密在订单页查看）。",
      "仅限美区账号使用；激活前请确认账号地区。已激活的卡密无法退换，未激活可在确认期内发起争议。",
      "批量购买请看 4 楼的批量报价，或私信我发指定买家报价。",
    ],
    likers: [3, 10, 5, 12],
    likeCount: 14,
    comments: [
      { id: 1, authorId: 6, text: "支持国区吗？" },
      { id: 2, authorId: 8, replyTo: 6, text: "只支持美区，国区的过几天上。" },
    ],
  },
  {
    floor: 2,
    authorId: 3,
    time: "昨天 21:40",
    timeIso: "2026-10-19T13:40:00Z",
    body: ["买过两次，付款后几秒就发卡了，激活正常。"],
    likers: [8, 10, 12],
    likeCount: 5,
    comments: [{ id: 1, authorId: 8, replyTo: 3, text: "感谢支持～" }],
  },
  {
    floor: 3,
    authorId: 10,
    time: "今天 10:05",
    timeIso: "2026-10-20T02:05:00Z",
    body: ["一次要 10 张能便宜点吗？"],
    likers: [],
    likeCount: 0,
    comments: [
      { id: 1, authorId: 8, replyTo: 10, text: "可以，私信你发了指定买家报价，楼下也贴了一份。" },
    ],
  },
  {
    floor: 4,
    authorId: 8,
    time: "今天 10:12",
    timeIso: "2026-10-20T02:12:00Z",
    body: ["批量 10 张的公开报价（单张 43 USDT），任何人都能直接担保购买："],
    hasOffer: true,
    likers: [10, 5],
    likeCount: 2,
    comments: [],
  },
];

export const auctionFloors: FloorData[] = [
  {
    floor: 1,
    authorId: 9,
    time: "前天 19:00",
    timeIso: "2026-10-18T11:00:00Z",
    body: [
      "自用收藏，未拆封带原盒，外盒有轻微货架磨痕（见图 3）。",
      "人民币计价，按我签名的汇率 7.2000 折合 USDT 结算；成交后 48 小时内付款，顺丰到付，3 天内发货。",
      "保留价已设，未达保留价流拍。本人不参与出价，也请勿用小号托价。",
    ],
    likers: [11, 3, 5, 12],
    likeCount: 9,
    comments: [{ id: 1, authorId: 12, text: "盒子保存得真好。" }],
  },
  {
    floor: 2,
    authorId: 11,
    time: "昨天 20:11",
    timeIso: "2026-10-19T12:11:00Z",
    body: ["盒角有没有压痕？能补一张侧面图吗？"],
    likers: [3],
    likeCount: 1,
    comments: [
      { id: 1, authorId: 9, replyTo: 11, text: "已补在 1 楼最后一张，盒角完好。" },
      { id: 2, authorId: 11, replyTo: 9, text: "看到了，谢谢，出价了。" },
    ],
  },
  {
    floor: 3,
    authorId: 3,
    time: "今天 09:30",
    timeIso: "2026-10-20T01:30:00Z",
    body: ["人民币计价挺方便的，出价时直接看到折合多少 USDT。"],
    likers: [12, 5],
    likeCount: 2,
    comments: [],
  },
];

export interface Bid {
  id: number;
  /** 已脱敏，如「星**」 */
  bidder: string;
  /** 拍卖币种最小单位 */
  amount: string;
  time: string;
  timeIso: string;
}

export const bids: Bid[] = [
  { id: 23, bidder: "北**", amount: "128000", time: "16:08:41", timeIso: "2026-10-20T08:08:41Z" },
  { id: 22, bidder: "夜**", amount: "126000", time: "16:02:10", timeIso: "2026-10-20T08:02:10Z" },
  { id: 21, bidder: "北**", amount: "124000", time: "15:47:55", timeIso: "2026-10-20T07:47:55Z" },
  { id: 20, bidder: "m***o", amount: "120000", time: "15:20:03", timeIso: "2026-10-20T07:20:03Z" },
  { id: 19, bidder: "张**", amount: "116000", time: "14:51:30", timeIso: "2026-10-20T06:51:30Z" },
  { id: 18, bidder: "夜**", amount: "110000", time: "13:05:12", timeIso: "2026-10-20T05:05:12Z" },
];

/** 交易板块：置顶规则帖 + 交易帖、拍卖帖 + 普通讨论 */
export const tradeBoardTopics: TopicSummary[] = [
  {
    id: 201,
    title: "交易板块规则：违禁品清单、担保流程与防骗指南（发帖前必读）",
    board: "trade",
    authorId: 1,
    participants: [1, 7],
    replies: 41,
    views: 15200,
    activity: "1 天",
    activityIso: "2026-10-19T08:00:00Z",
    pinned: true,
    locked: true,
    href: "/design/v2/topic",
  },
  ...topics.filter((t) => t.trade || t.auction),
  {
    id: 202,
    title: "交易经验：自动发卡的库存怎么设，卖完会自动下架吗？",
    board: "trade",
    authorId: 12,
    participants: [12, 8, 1],
    replies: 8,
    views: 290,
    activity: "50 分钟",
    activityIso: "2026-10-20T07:20:00Z",
    featured: true,
    href: "/design/v2/topic",
  },
];
