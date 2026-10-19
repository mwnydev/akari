import { ApiError } from "@akari/api-client";
import { describe, expect, it } from "vitest";
import { createWalletApi, userMessage } from "./api";

const BASE = "http://forum.test";

interface Seen {
  url: string;
  method: string;
  credentials: RequestCredentials | undefined;
  body: unknown;
}

/** 假的 forum-api：记下请求，按路径返回固定响应。 */
function fakeApi(routes: Record<string, () => Response>) {
  const seen: Seen[] = [];
  const f: typeof fetch = async (input, init) => {
    const req = input instanceof Request ? input : new Request(input, init);
    const text = await req.text();
    seen.push({
      url: req.url,
      method: req.method,
      credentials: input instanceof Request ? input.credentials : init?.credentials,
      body: text ? JSON.parse(text) : undefined,
    });
    const route = routes[`${req.method} ${new URL(req.url).pathname}${new URL(req.url).search}`];
    if (!route) throw new TypeError("Failed to fetch");
    return route();
  };
  return { api: createWalletApi({ baseUrl: BASE, fetch: f }), seen };
}

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("真实 WalletApi", () => {
  it("注册参数去掉 webauthn-rs 外层的 publicKey，请求带论坛会话 Cookie", async () => {
    const { api, seen } = fakeApi({
      "POST /api/v1/wallet/passkey/register/start": () =>
        json(200, { options: { publicKey: { challenge: "abc", rp: { id: "localhost" } } } }),
    });
    expect(await api.registrationOptions()).toEqual({
      challenge: "abc",
      rp: { id: "localhost" },
    });
    expect(seen[0]?.credentials).toBe("include");
  });

  it("登记地址时带上初始化参数版本", async () => {
    const wallet = { chain_id: 31337, address: `0x${"ab".repeat(20)}`, init_version: 2 };
    const { api, seen } = fakeApi({
      "POST /api/v1/wallet/account": () =>
        json(201, { ...wallet, passkey_id: 1, created_at: "2026-10-19T00:00:00Z" }),
    });
    await api.bindWallet(wallet);
    expect(seen[0]?.body).toEqual(wallet);
  });

  it("后端错误码变成带中文提示的 ApiError", async () => {
    const { api } = fakeApi({
      "POST /api/v1/offers/5/accept": () =>
        json(409, { code: "OFFER_UNAVAILABLE", message: "报价已失效" }),
    });
    const err = await api.acceptOffer(5).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: "OFFER_UNAVAILABLE", status: 409 });
    expect(userMessage(err, "兜底")).toBe("报价已失效");
  });

  it("204 没有响应体也算成功", async () => {
    const { api, seen } = fakeApi({
      "POST /api/v1/wallet/reverify/verify": () => new Response(null, { status: 204 }),
    });
    await expect(api.verifyReverify("123456")).resolves.toBeUndefined();
    expect(seen[0]?.body).toEqual({ code: "123456" });
  });

  it("网络不通时提示网络异常", async () => {
    const { api } = fakeApi({});
    const err = await api.overview().catch((e: unknown) => e);
    expect(err).toMatchObject({ code: "NETWORK" });
    expect(userMessage(err, "兜底")).toBe("网络异常，请稍后再试");
  });

  it("订单接口：路径、游标参数与错误码", async () => {
    const page = { orders: [], next_cursor: null };
    const { api, seen } = fakeApi({
      "GET /api/v1/orders?role=seller&cursor=41": () => json(200, page),
      "GET /api/v1/orders?role=buyer": () => json(200, page),
      "GET /api/v1/orders/9": () =>
        json(404, { code: "NOT_FOUND", message: "内容不存在或已被删除" }),
    });
    expect(await api.orders("seller", 41)).toEqual(page);
    expect(await api.orders("buyer")).toEqual(page);
    expect(seen[0]?.credentials).toBe("include");
    await expect(api.order(9)).rejects.toMatchObject({ code: "NOT_FOUND", status: 404 });
  });

  it("拍卖：开始签名、提交签名带 Cookie，路径与请求体与后端一致", async () => {
    const signing = { auction_id: 7, auth_hash: `0x${"ab".repeat(32)}` };
    const { api, seen } = fakeApi({
      "POST /api/v1/auctions/7/start": () => json(200, signing),
      "POST /api/v1/auctions/7/finish": () => json(200, { id: 7, topic_id: 12 }),
      "GET /api/v1/auctions/7/payment": () =>
        json(403, { code: "FORBIDDEN", message: "没有权限进行此操作" }),
    });
    expect(await api.startAuction(7)).toEqual(signing);
    expect(await api.finishAuction(7, "0x01")).toMatchObject({ topic_id: 12 });
    expect(seen.map((r) => [r.method, r.body])).toEqual([
      ["POST", undefined],
      ["POST", { signature: "0x01" }],
    ]);
    expect(seen.every((r) => r.credentials === "include")).toBe(true);
    await expect(api.auctionPayment(7)).rejects.toMatchObject({ code: "FORBIDDEN", status: 403 });
  });

  it("不是后端错误格式的失败响应按网络异常处理", async () => {
    const { api } = fakeApi({
      "GET /api/v1/offers/3/order": () => new Response("bad gateway", { status: 502 }),
    });
    await expect(api.offerOrder(3)).rejects.toMatchObject({ code: "NETWORK", status: 502 });
  });
});
