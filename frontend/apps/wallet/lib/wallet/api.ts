import { ApiError, createApi, errorMessage, type Schemas, type paths } from "@akari/api-client";
import type {
  AuthenticationResponseJSON,
  PublicKeyCredentialCreationOptionsJSON,
  PublicKeyCredentialRequestOptionsJSON,
  RegistrationResponseJSON,
} from "@simplewebauthn/browser";
import { API_BASE } from "./config";

export type PasskeyView = Schemas["PasskeyView"];
export type ApiOffer = Schemas["Offer"];
export type OfferView = Schemas["OfferView"];
export type NewOfferRequest = Schemas["NewOfferRequest"];
export type PaymentData = Schemas["PaymentData"];
export type ListingView = Schemas["ListingView"];
export type ReverifyStarted = Schemas["ReverifyStarted"];
export type Verified = Schemas["Verified"];
export type WalletView = Schemas["WalletView"];
export type WalletOverview = Schemas["WalletOverview"];
export type NewWallet = Schemas["NewWallet"];
export type TradeParams = Schemas["TradeParams"];
export type WalletInitVersionView = Schemas["WalletInitVersionView"];
export type OrderView = Schemas["OrderView"];
export type OrderPage = Schemas["OrderPage"];
export type OrderRole = paths["/api/v1/orders"]["get"]["parameters"]["query"]["role"];
/** 钱包余额（最小单位十进制字符串）、读取时的区块号与免费代付提币次数。 */
export type WalletBalances = Schemas["Balances"];
export type WithdrawAddressView = Schemas["WithdrawAddressView"];
export type NewWithdrawAddress = Schemas["NewWithdrawAddressRequest"];

/**
 * 钱包前端依赖的全部后端接口。页面与 SDK 只依赖这个接口；单元测试换成内存实现（见 ./sdk.test.ts）。
 * 所有方法失败时抛 `ApiError`（message 已是中文，可直接显示）。
 */
export interface WalletApi {
  /** 我的钱包 Passkey、钱包地址、近期 Passkey 验证截止时间；未登录抛 UNAUTHORIZED */
  overview(): Promise<WalletOverview>;
  tradeParams(): Promise<TradeParams>;
  /** 开通交易第 1 步：向账号邮箱发「开通交易」验证码 */
  startReverify(): Promise<ReverifyStarted>;
  verifyReverify(code: string): Promise<void>;
  /** 第 2 步：后端出 challenge（Valkey 5 分钟）→ 浏览器创建 Passkey → 后端 webauthn-rs 验证并保存 */
  registrationOptions(): Promise<PublicKeyCredentialCreationOptionsJSON>;
  finishRegistration(credential: RegistrationResponseJSON): Promise<PasskeyView>;
  /** 敏感操作前的 Passkey 再验证（有效 5 分钟） */
  assertionOptions(): Promise<PublicKeyCredentialRequestOptionsJSON>;
  finishAssertion(credential: AuthenticationResponseJSON): Promise<Verified>;
  /** 第 3 步：前端算出的智能账户地址写入 wallets 表（需近期 Passkey 验证；注册成功本身算一次） */
  bindWallet(wallet: NewWallet): Promise<WalletView>;
  listing(id: number): Promise<ListingView>;
  /** 提交卖家签好的报价卡片（需近期 Passkey 验证） */
  createOffer(req: NewOfferRequest): Promise<OfferView>;
  /** 买家接受卡片，拿到付款数据 */
  acceptOffer(id: number): Promise<PaymentData>;
  /** `id` 为订单的站内 ID（`OrderView.id`），不是链上订单号（链上订单号按合约各自编号） */
  order(id: number): Promise<OrderView>;
  orders(role: OrderRole, cursor?: number): Promise<OrderPage>;
  /** 某张报价卡片付款后生成的订单（indexer 同步到之前为 NOT_FOUND） */
  offerOrder(offerId: number): Promise<OrderView>;
  /** 钱包的 USDT 与 ETH 余额（后端读链并缓存） */
  balances(): Promise<WalletBalances>;
  /** 保存的提币地址（含冷却中与已撤销的） */
  withdrawAddresses(): Promise<WithdrawAddressView[]>;
  /** 新增提币地址（需近期 Passkey 验证）：第一个立即生效，之后的 24 小时后生效 */
  addWithdrawAddress(req: NewWithdrawAddress): Promise<WithdrawAddressView>;
  /** 撤销提币地址（需近期 Passkey 验证） */
  removeWithdrawAddress(id: number): Promise<void>;
}

interface Options {
  /** 默认 `API_BASE`（同源时为空串） */
  baseUrl?: string;
  fetch?: typeof fetch;
}

interface FetchResult<T> {
  data?: T;
  error?: unknown;
  response: Response;
}

/** openapi-fetch 的结果 → 数据或 ApiError。网络层失败（fetch 抛错）也统一成 ApiError。 */
async function unwrap<T>(pending: Promise<FetchResult<T>>): Promise<T> {
  let result: FetchResult<T>;
  try {
    result = await pending;
  } catch {
    throw ApiError.from(undefined, 0);
  }
  const { data, error, response } = result;
  if (!response.ok) throw ApiError.from(error, response.status);
  return data as T;
}

/** webauthn-rs 的 `CreationChallengeResponse` / `RequestChallengeResponse` 外面包着一层 `publicKey`。 */
function publicKeyOf<T>(options: unknown): T {
  const inner = (options as { publicKey?: T } | null)?.publicKey;
  if (!inner) throw new Error("Passkey 参数不完整，请刷新后重试");
  return inner;
}

export function createWalletApi({ baseUrl = API_BASE, fetch: f = fetch }: Options = {}): WalletApi {
  // 钱包子域名跨源调用论坛 API，必须带上论坛会话 Cookie（ADR 0009）
  const api = createApi({ baseUrl, credentials: "include", fetch: (req) => f(req) });

  return {
    overview: () => unwrap(api.GET("/api/v1/wallet")),
    tradeParams: () => unwrap(api.GET("/api/v1/trade/params")),
    startReverify: () => unwrap(api.POST("/api/v1/wallet/reverify/start")),
    async verifyReverify(code) {
      await unwrap(api.POST("/api/v1/wallet/reverify/verify", { body: { code } }));
    },
    async registrationOptions() {
      const { options } = await unwrap(api.POST("/api/v1/wallet/passkey/register/start"));
      return publicKeyOf(options);
    },
    finishRegistration: (credential) =>
      unwrap(
        api.POST("/api/v1/wallet/passkey/register/finish", {
          body: { credential: credential as unknown as Record<string, never> },
        }),
      ),
    async assertionOptions() {
      const { options } = await unwrap(api.POST("/api/v1/wallet/passkey/assert/start"));
      return publicKeyOf(options);
    },
    finishAssertion: (credential) =>
      unwrap(
        api.POST("/api/v1/wallet/passkey/assert/finish", {
          body: { credential: credential as unknown as Record<string, never> },
        }),
      ),
    bindWallet: (wallet) => unwrap(api.POST("/api/v1/wallet/account", { body: wallet })),
    listing: (id) => unwrap(api.GET("/api/v1/listings/{id}", { params: { path: { id } } })),
    createOffer: (req) => unwrap(api.POST("/api/v1/offers", { body: req })),
    acceptOffer: (id) =>
      unwrap(api.POST("/api/v1/offers/{id}/accept", { params: { path: { id } } })),
    order: (id) => unwrap(api.GET("/api/v1/orders/{id}", { params: { path: { id } } })),
    orders: (role, cursor) =>
      unwrap(api.GET("/api/v1/orders", { params: { query: { role, cursor: cursor ?? null } } })),
    offerOrder: (id) => unwrap(api.GET("/api/v1/offers/{id}/order", { params: { path: { id } } })),
    balances: () => unwrap(api.GET("/api/v1/wallet/balances")),
    async withdrawAddresses() {
      const { addresses } = await unwrap(api.GET("/api/v1/wallet/withdraw-addresses"));
      return addresses;
    },
    addWithdrawAddress: (req) =>
      unwrap(api.POST("/api/v1/wallet/withdraw-addresses", { body: req })),
    async removeWithdrawAddress(id) {
      await unwrap(
        api.DELETE("/api/v1/wallet/withdraw-addresses/{id}", { params: { path: { id } } }),
      );
    },
  };
}

/** 给用户看的一句话：接口错误用后端的中文提示，SDK 抛出的中文提示原样显示，其他用兜底文案。 */
export function userMessage(e: unknown, fallback: string): string {
  if (e instanceof ApiError) return errorMessage(e);
  return e instanceof Error && /[一-鿿]/.test(e.message) ? e.message : fallback;
}

let instance: WalletApi | undefined;

export function walletApi(): WalletApi {
  instance ??= createWalletApi();
  return instance;
}
