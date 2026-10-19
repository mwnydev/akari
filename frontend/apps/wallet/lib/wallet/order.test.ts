import { describe, expect, it } from "vitest";
import type { OrderView } from "./api";
import {
  availableAction as action,
  isFinal,
  levelBadge,
  onceState,
  remaining,
  stateText,
} from "./order";

const BUYER = "0x1111111111111111111111111111111111111111";
const SELLER = "0x2222222222222222222222222222222222222222";
const NOW = Date.parse("2026-10-19T00:00:00Z");
const ESCROW = "0xFb3ca740dac6A93a5110a0F8d9cc584ACE80940C";
const availableAction = (o: OrderView, me: string, now: number) => action(o, me, now, ESCROW);

const order = (o: Partial<OrderView> = {}, deadlines: OrderView["deadlines"] = {}): OrderView => ({
  id: 7,
  chain_id: 31337,
  contract: ESCROW,
  chain_order_id: 3,
  mode: "escrow",
  buyer: { address: BUYER, user_id: 1 },
  seller: { address: SELLER, user_id: 2 },
  token: "0x8c313D0eC280837325fA5aB05AC04Be31F46AbBC",
  amount: "20000000",
  fees: {},
  instant: false,
  state: "funded",
  deadlines: { ship: "2026-10-22T00:00:00Z", confirm: null, ...deadlines },
  level: "latest",
  source: { offer_hash: `0x${"00".repeat(32)}`, listing_hash: `0x${"00".repeat(32)}` },
  created_block: 10,
  updated_block: 10,
  created_at: "2026-10-19T00:00:00Z",
  updated_at: "2026-10-19T00:00:00Z",
  ...o,
});
const DELIVERED = { confirm: "2026-10-25T00:00:00Z" };
const NOW_ISO = "2026-10-19T00:00:00Z";

describe("订单页规则", () => {
  it("已付款且有确认期限 = 卖家已发货；回滚后的订单没有进度", () => {
    expect(onceState(order())).toBe("funded");
    expect(onceState(order({}, DELIVERED))).toBe("delivered");
    expect(onceState(order({ state: "released" }))).toBe("released");
    expect(onceState(order({ state: "none" }))).toBeNull();
  });

  it("latest / safe 显示「已上链（处理中）」，finalized 显示最终结果", () => {
    expect(levelBadge(order()).label).toBe("已上链（处理中）");
    expect(levelBadge(order({ level: "safe" })).label).toBe("已上链（处理中）");
    expect(levelBadge(order({ level: "finalized" })).label).toBe("已最终确认");
  });

  it("只有结局已定且最终确认才停止刷新", () => {
    expect(isFinal(order({ state: "released", level: "finalized" }))).toBe(true);
    expect(isFinal(order({ state: "released", level: "safe" }))).toBe(false);
    expect(isFinal(order({ state: "funded", level: "finalized" }))).toBe(false);
  });

  it("按钮按角色与状态出现，地址大小写无关", () => {
    expect(availableAction(order(), SELLER.toUpperCase().replace("0X", "0x"), NOW)?.kind).toBe(
      "markDelivered",
    );
    expect(availableAction(order(), BUYER, NOW)?.kind).toBe("release");
    expect(availableAction(order(), "0x3333333333333333333333333333333333333333", NOW)).toBeNull();
  });

  it("卖家已发货或过了发货期限后不能再标记发货；非已付款状态谁都没有按钮", () => {
    const delivered = order({}, DELIVERED);
    expect(availableAction(delivered, SELLER, NOW)).toBeNull();
    expect(availableAction(delivered, BUYER, NOW)?.kind).toBe("release");
    expect(availableAction(order({}, { ship: "2026-10-18T00:00:00Z" }), SELLER, NOW)).toBeNull();
    for (const state of ["disputed", "released", "refunded", "settled"] as const) {
      expect(availableAction(order({ state }), BUYER, NOW)).toBeNull();
      expect(availableAction(order({ state }), SELLER, NOW)).toBeNull();
    }
  });

  it("分期订单与别的合约上的订单没有一次性担保的按钮", () => {
    const stream = order({
      mode: "stream",
      contract: "0x6022875397e11A32BbAEB91d80258CBF4AA6F4A8",
    });
    expect(availableAction(stream, BUYER, NOW)).toBeNull();
    expect(availableAction(order({ contract: SELLER }), BUYER, NOW)).toBeNull();
    expect(availableAction(order({ contract: ESCROW.toLowerCase() }), BUYER, NOW)?.kind).toBe(
      "release",
    );
  });

  it("状态文案区分一次性与分期", () => {
    expect(stateText(order({}, DELIVERED))).toBe("卖家已发货，等待买家确认收货");
    expect(stateText(order({ state: "none" }))).toBeNull();
    const stream = (o: Partial<OrderView>) => order({ mode: "stream", ...o });
    expect(stateText(stream({}))).toBe("买家已付款，等待卖家开始服务");
    const started = { interval_secs: 86_400, periods: 30, claimed: "0", started_at: NOW_ISO };
    expect(stateText(stream({ stream: started }))).toBe("服务进行中，按期解锁给卖家");
    expect(stateText(stream({ state: "released" }))).toBe("已全部解锁给卖家");
  });

  it("期限倒计时", () => {
    expect(remaining("2026-10-21T03:00:00Z", NOW)).toBe("还剩 2 天 3 小时");
    expect(remaining("2026-10-19T05:30:00Z", NOW)).toBe("还剩 5 小时 30 分");
    expect(remaining("2026-10-19T00:00:20Z", NOW)).toBe("还剩 1 分钟");
    expect(remaining("2026-10-18T00:00:00Z", NOW)).toBe("已到期");
  });
});
