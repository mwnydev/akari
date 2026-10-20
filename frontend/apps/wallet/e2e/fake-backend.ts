import { escrowAbi, localChain as chain, mockUsdtAbi } from "@akari/contracts";
import type { BrowserContext, Route } from "@playwright/test";
import { createPublicKey, randomBytes } from "node:crypto";
import type {
  ApiAuctionAuth,
  AuctionPayment,
  AuctionSigning,
  AuctionView,
  NewOfferRequest,
  NewWallet,
  OfferView,
  OrderView,
  PasskeyView,
  PaymentData,
  TradeParams,
  WalletBalances,
  WalletOverview,
  WalletView,
  WithdrawAddressView,
} from "../lib/wallet/api";
import { parseAuctionAuth } from "../lib/wallet/auction";
import { toView } from "../lib/wallet/init-version";
import { AUCTION_TYPES } from "../lib/wallet/typed-data";
import { publicClient } from "./env";

// 测试替身：forum-api 没运行、或还没实现某些接口（订单接口由 indexer 提供）时，在浏览器网络层模拟它们。
// 形状取自钱包前端用的同一套类型（lib/wallet/api.ts）；只做流程需要的校验，不验 WebAuthn 签名
// （真实校验在后端 webauthn-rs，CDP 虚拟认证器保证这里拿到的是合法凭证）。

/** 开通交易邮件里的验证码（模拟） */
export const FAKE_EMAIL_CODE = "123456";
const RECENT_SECONDS = 300;
/** 第二个起的提币地址冷却 24 小时（plan 第 5 节） */
const ADDRESS_COOLDOWN_MS = 86_400_000;
const STATES = ["none", "funded", "disputed", "released", "refunded", "settled"];

interface User {
  id: number;
  name: string;
  reverified: boolean;
  challenge?: string | undefined;
  passkeys: PasskeyView[];
  wallets: WalletView[];
  verifiedUntil: number | null;
  verifiedPasskey?: number;
  addresses: WithdrawAddressView[];
}

const now = () => Math.floor(Date.now() / 1000);
const b64url = (b: Buffer) => b.toString("base64url");
const hex = (b64: string) => `0x${Buffer.from(b64, "base64url").toString("hex")}`;
const iso = (secs: bigint) => (secs === 0n ? null : new Date(Number(secs) * 1000).toISOString());

class HttpError {
  constructor(
    readonly status: number,
    readonly code: string,
    readonly message: string,
  ) {}
}

/** 浏览器交过来的 WebAuthn 结果里 clientDataJSON 的 challenge 必须是我们发的那个。 */
function checkChallenge(user: User, credential: { response: { clientDataJSON: string } }) {
  const data = JSON.parse(Buffer.from(credential.response.clientDataJSON, "base64url").toString());
  if (!user.challenge || data.challenge !== user.challenge) {
    throw new HttpError(400, "PASSKEY_FAILED", "Passkey 验证失败，请重试");
  }
  user.challenge = undefined;
}

interface FakeAuction {
  /** 卖家看到的拍卖（含保留价）；上线后 state 变 live、带上拍卖帖主题 */
  view: AuctionView;
  /** `POST /auctions/{id}/start` 生成的授权 */
  auth: ApiAuctionAuth;
  /** 上线后的拍卖帖主题 */
  topicId: number;
  /** 卖家在钱包签好后提交的授权签名 */
  sellerSig?: string;
  payment?: AuctionPayment;
}

export class FakeBackend {
  private users: User[] = [];
  private offers: OfferView[] = [];
  private finalized = new Set<string>();
  private auctions = new Map<string, FakeAuction>();

  /**
   * 测试登记一场待卖家签名的拍卖草稿（真实的挂拍、出价、成交证明与 createFromAuction 见 forum-api 的
   * `auction_e2e.rs`；这里只测钱包页面与链上的那一段）。
   */
  addAuction(view: AuctionView, auth: ApiAuctionAuth, topicId: number) {
    this.auctions.set(String(view.id), { view, auth, topicId });
  }

  /** 测试登记过的拍卖的接口总由这里回答（有真实 forum-api 时也一样：真实后端里没有这场拍卖）。 */
  servesAuction(path: string): boolean {
    const id = /^\/api\/v1\/auctions\/(\d+)(?:\/|$)/.exec(path)?.[1];
    return id !== undefined && this.auctions.has(id);
  }

  auctionSellerSig(id: number): string | undefined {
    return this.auctions.get(String(id))?.sellerSig;
  }

  /** 结束后由测试签好成交证明（模拟 forum-sponsor），赢家付款页从这里取数据。 */
  settleAuction(id: number, payment: Omit<AuctionPayment, "auth" | "seller_signature">) {
    const a = this.auctions.get(String(id));
    if (!a?.sellerSig) throw new Error(`auction ${id} has no seller signature`);
    a.payment = { ...payment, auth: a.auth, seller_signature: a.sellerSig };
    a.view = { ...a.view, state: "awaiting_payment", round: 1 };
  }

  private auction(id: string): FakeAuction {
    const a = this.auctions.get(id);
    if (!a) throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
    return a;
  }

  /** 拍卖详情；付款上链后（模拟 indexer）带上站内订单 ID = 链上订单号。 */
  private async auctionView(a: FakeAuction): Promise<AuctionView> {
    if (!a.payment) return a.view;
    const logs = await publicClient.getContractEvents({
      address: chain.contracts.Escrow,
      abi: escrowAbi,
      eventName: "AuctionSettled",
      args: { auctionId: BigInt(a.view.id) },
      fromBlock: (await publicClient.getBlockNumber()) - 1000n,
    });
    const orderId = logs.at(-1)?.args.orderId;
    return orderId ? { ...a.view, state: "settled", order_id: Number(orderId) } : a.view;
  }

  /** 让某个订单在模拟的 indexer 里变成 finalized（真实环境要等十几分钟）。 */
  finalize(orderId: string) {
    this.finalized.add(orderId);
  }

  /**
   * 给浏览器上下文接上模拟后端，上下文 = 一个已登录用户。
   * `serve` 判断某个接口要不要由这里回答；不要的照常发给真实 forum-api。
   */
  async attach(context: BrowserContext, serve: (path: string, method: string) => boolean) {
    const user: User = {
      id: this.users.length + 1,
      name: `e2e-${this.users.length + 1}`,
      reverified: false,
      passkeys: [],
      wallets: [],
      verifiedUntil: null,
      addresses: [],
    };
    this.users.push(user);
    await context.route("**/api/v1/**", async (route) => {
      const req = route.request();
      const path = new URL(req.url()).pathname;
      if (!serve(path, req.method())) return route.continue();
      try {
        const [status, body] = await this.handle(user, req.method(), path, req.postDataJSON());
        await reply(route, status, body);
      } catch (e) {
        if (!(e instanceof HttpError)) throw e;
        await reply(route, e.status, { code: e.code, message: e.message });
      }
    });
  }

  private recent(user: User): number {
    if (!user.verifiedPasskey || !user.verifiedUntil || user.verifiedUntil < now()) {
      throw new HttpError(403, "PASSKEY_REQUIRED", "请先用钱包 Passkey 确认");
    }
    return user.verifiedPasskey;
  }

  private markVerified(user: User, passkeyId: number) {
    user.verifiedPasskey = passkeyId;
    user.verifiedUntil = now() + RECENT_SECONDS;
  }

  private walletOf(user: User): WalletView {
    const w = user.wallets[0];
    if (!w) throw new HttpError(403, "WALLET_REQUIRED", "请先开通交易");
    return w;
  }

  private async handle(
    user: User,
    method: string,
    path: string,
    body: unknown,
  ): Promise<[number, unknown]> {
    const route = `${method} ${path.replace(/\/\d+(?=\/|$)/g, "/{id}")}`;
    const id = /\/(\d+)(?:\/|$)/.exec(path)?.[1] ?? "";
    switch (route) {
      case "GET /api/v1/wallet": {
        const overview: WalletOverview = {
          enabled: user.passkeys.length > 0 && user.wallets.length > 0,
          passkeys: user.passkeys,
          wallets: user.wallets,
          verified_until: user.verifiedUntil,
          needs_username: false,
        };
        return [200, overview];
      }
      case "GET /api/v1/trade/params":
        return [200, tradeParams()];
      case "POST /api/v1/wallet/reverify/start":
        return [202, { expires_in: 600, resend_in: 60 }];
      case "POST /api/v1/wallet/reverify/verify":
        if ((body as { code: string }).code !== FAKE_EMAIL_CODE) {
          throw new HttpError(400, "INVALID_CODE", "验证码错误或已过期");
        }
        user.reverified = true;
        return [204, null];
      case "POST /api/v1/wallet/passkey/register/start": {
        if (!user.reverified) throw new HttpError(403, "REVERIFY_REQUIRED", "请先完成邮箱验证");
        user.challenge = b64url(randomBytes(32));
        const name = `Akari 钱包签名（勿删）· ${user.name}`;
        const publicKey = {
          challenge: user.challenge,
          rp: { id: "localhost", name: "Akari" },
          user: { id: b64url(Buffer.from(String(user.id))), name, displayName: name },
          pubKeyCredParams: [{ type: "public-key", alg: -7 }],
          authenticatorSelection: { residentKey: "required", userVerification: "required" },
          attestation: "none",
          timeout: 60_000,
        };
        return [200, { options: { publicKey } }];
      }
      case "POST /api/v1/wallet/passkey/register/finish": {
        const { credential } = body as {
          credential: { id: string; response: { clientDataJSON: string; publicKey: string } };
        };
        checkChallenge(user, credential);
        user.reverified = false; // 一次性
        const jwk = createPublicKey({
          key: Buffer.from(credential.response.publicKey, "base64url"),
          format: "der",
          type: "spki",
        }).export({ format: "jwk" });
        const passkey: PasskeyView = {
          id: user.id * 100 + user.passkeys.length + 1,
          credential_id: credential.id,
          pubkey_x: hex(jwk.x ?? ""),
          pubkey_y: hex(jwk.y ?? ""),
          created_at: new Date().toISOString(),
        };
        user.passkeys.push(passkey);
        this.markVerified(user, passkey.id);
        return [201, passkey];
      }
      case "POST /api/v1/wallet/passkey/assert/start": {
        user.challenge = b64url(randomBytes(32));
        const publicKey = {
          challenge: user.challenge,
          rpId: "localhost",
          allowCredentials: user.passkeys.map((p) => ({ type: "public-key", id: p.credential_id })),
          userVerification: "required",
        };
        return [200, { options: { publicKey } }];
      }
      case "POST /api/v1/wallet/passkey/assert/finish": {
        const { credential } = body as {
          credential: { id: string; response: { clientDataJSON: string } };
        };
        checkChallenge(user, credential);
        const passkey = user.passkeys.find((p) => p.credential_id === credential.id);
        if (!passkey) throw new HttpError(400, "PASSKEY_FAILED", "Passkey 验证失败，请重试");
        this.markVerified(user, passkey.id);
        return [200, { passkey_id: passkey.id, verified_until: user.verifiedUntil }];
      }
      case "POST /api/v1/wallet/account": {
        const req = body as NewWallet;
        const wallet: WalletView = {
          ...req,
          passkey_id: this.recent(user),
          created_at: new Date().toISOString(),
        };
        user.wallets.push(wallet);
        return [201, wallet];
      }
      case "GET /api/v1/wallet/balances": {
        const address = this.walletOf(user).address as `0x${string}`;
        const [usdt, eth, block, transfers] = await Promise.all([
          publicClient.readContract({
            address: chain.contracts.MockUSDT,
            abi: mockUsdtAbi,
            functionName: "balanceOf",
            args: [address],
          }),
          publicClient.getBalance({ address }),
          publicClient.getBlockNumber(),
          publicClient.getContractEvents({
            address: chain.contracts.MockUSDT,
            abi: mockUsdtAbi,
            eventName: "Transfer",
            args: { from: address },
            fromBlock: (await publicClient.getBlockNumber()) - 1000n,
          }),
        ]);
        // 模拟次数：每个钱包送 1 次、每天 3 次；钱包转给自己保存的提币地址算用掉一次（不模拟完成订单 +1）
        const saved = new Set(user.addresses.map((a) => a.address.toLowerCase()));
        const used = transfers.filter((t) => saved.has(String(t.args.to).toLowerCase())).length;
        const balances: WalletBalances = {
          usdt: usdt.toString(),
          eth: eth.toString(),
          block: Number(block),
          withdraw_credits: Math.max(0, 1 - used),
          withdraws_left_today: Math.max(0, 3 - used),
        };
        return [200, balances];
      }
      case "GET /api/v1/wallet/withdraw-addresses":
        return [200, { addresses: user.addresses }];
      case "POST /api/v1/wallet/withdraw-addresses": {
        this.recent(user);
        const req = body as { label: string; address: string };
        const first = !user.addresses.some((a) => a.status !== "revoked");
        const created = new Date();
        const address: WithdrawAddressView = {
          id: user.id * 1000 + user.addresses.length + 1,
          label: req.label,
          address: req.address,
          status: first ? "active" : "pending",
          active_at: new Date(created.getTime() + (first ? 0 : ADDRESS_COOLDOWN_MS)).toISOString(),
          created_at: created.toISOString(),
        };
        user.addresses.push(address);
        return [201, address];
      }
      case "DELETE /api/v1/wallet/withdraw-addresses/{id}": {
        this.recent(user);
        const address = user.addresses.find((a) => a.id === Number(id));
        if (!address) throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
        address.status = "revoked";
        return [204, null];
      }
      case "POST /api/v1/offers": {
        this.recent(user);
        const req = body as NewOfferRequest;
        if (req.offer.seller.toLowerCase() !== this.walletOf(user).address.toLowerCase()) {
          throw new HttpError(403, "FORBIDDEN", "没有权限进行此操作");
        }
        const offer: OfferView = {
          id: this.offers.length + 1,
          conversation_id: req.conversation_id ?? null,
          listing_id: req.listing_id ?? null,
          seller_id: user.id,
          status: "pending",
          chain_id: chain.chainId,
          verifying_contract: chain.contracts.Escrow,
          offer: req.offer,
          signature: req.signature,
          offer_hash: `0x${"00".repeat(32)}`,
          expires_at: new Date(req.offer.expiry * 1000).toISOString(),
          created_at: new Date().toISOString(),
        };
        this.offers.push(offer);
        return [201, offer];
      }
      case "POST /api/v1/offers/{id}/accept": {
        const card = this.offers.find((o) => o.id === Number(id));
        if (!card) throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
        const payment: PaymentData = {
          offer_id: card.id,
          escrow: card.verifying_contract,
          chain_id: card.chain_id,
          offer: card.offer,
          signature: card.signature,
          offer_hash: card.offer_hash,
          // 前端以链上参数为准，这几项只是展示用的估算
          buyer_pays: card.offer.amount,
          buyer_fee: "0",
          buyer_gas: "0",
          buyer_wallet: this.walletOf(user).address,
        };
        return [200, payment];
      }
      case "GET /api/v1/orders/{id}":
        return [200, await this.order(id)];
      case "GET /api/v1/offers/{id}/order": {
        // 不记报价哈希：取这位买家最近一次建单（测试里每个买家只付一次）
        const logs = await publicClient.getContractEvents({
          address: chain.contracts.Escrow,
          abi: escrowAbi,
          eventName: "OrderCreated",
          args: { buyer: this.walletOf(user).address as `0x${string}` },
          fromBlock: (await publicClient.getBlockNumber()) - 1000n,
        });
        const created = logs.at(-1);
        if (!created?.args.orderId) throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
        return [200, await this.order(created.args.orderId.toString())];
      }
      case "GET /api/v1/auctions/{id}":
        return [200, await this.auctionView(this.auction(id))];
      case "POST /api/v1/auctions/{id}/start": {
        // 近期 Passkey 验证由真实后端检查（auction_flow.rs）；这里的用户可能在真实后端验证，不在这里记录
        const a = this.auction(id);
        const auth = parseAuctionAuth(a.auth);
        if (!auth || a.view.state !== "draft") {
          throw new HttpError(409, "AUCTION_UNAVAILABLE", "拍卖当前状态不能进行这项操作");
        }
        const authHash = await publicClient.readContract({
          address: chain.contracts.Escrow,
          abi: escrowAbi,
          functionName: "hashAuction",
          args: [auth],
        });
        const signing: AuctionSigning = {
          auction_id: a.view.id,
          typed_data: {
            domain: {
              name: "AkariEscrow",
              version: "1",
              chainId: chain.chainId,
              verifyingContract: chain.contracts.Escrow,
            },
            types: AUCTION_TYPES,
            primaryType: "AuctionAuth",
            message: a.auth,
          },
          auth_hash: authHash,
          sign_deadline: new Date(Date.now() + 600_000).toISOString(),
        };
        return [200, signing];
      }
      case "POST /api/v1/auctions/{id}/finish": {
        const a = this.auction(id);
        a.sellerSig = (body as { signature: string }).signature;
        a.view = { ...a.view, state: "live", topic_id: a.topicId };
        return [200, a.view];
      }
      case "GET /api/v1/auctions/{id}/payment": {
        const payment = this.auction(id).payment;
        if (!payment) throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
        return [200, payment];
      }
      default:
        throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
    }
  }

  /** 模拟 indexer：直接读链上 getOrder（站内订单 ID 就用 Escrow 的链上订单号）；确认级别由 `finalize` 控制。 */
  private async order(id: string): Promise<OrderView> {
    const o = await publicClient.readContract({
      address: chain.contracts.Escrow,
      abi: escrowAbi,
      functionName: "getOrder",
      args: [BigInt(id)],
    });
    if (o.state === 0) throw new HttpError(404, "NOT_FOUND", "内容不存在或已被删除");
    const now = new Date().toISOString();
    return {
      id: Number(id),
      chain_id: chain.chainId,
      contract: chain.contracts.Escrow,
      chain_order_id: Number(id),
      mode: "escrow",
      buyer: { address: o.buyer },
      seller: { address: o.seller },
      token: o.token,
      amount: o.amount.toString(),
      fees: {},
      instant: o.shipDeadline === 0n,
      state: STATES[o.state] ?? "none",
      deadlines: { ship: iso(o.shipDeadline), confirm: iso(o.confirmDeadline) },
      level: this.finalized.has(id) ? "finalized" : "latest",
      source: { offer_hash: `0x${"00".repeat(32)}`, listing_hash: `0x${"00".repeat(32)}` },
      created_block: 0,
      updated_block: 0,
      created_at: now,
      updated_at: now,
    };
  }
}

function tradeParams(): TradeParams {
  return {
    domain: {
      name: "AkariEscrow",
      version: "1",
      chainId: chain.chainId,
      verifyingContract: chain.contracts.Escrow,
    },
    token: chain.contracts.MockUSDT,
    min_order: "5000000",
    order_cap: "200000000",
    fee_bps: 100,
    min_fee: "1000000",
    base_gas: "100000",
    min_window: 86_400,
    max_window: 30 * 86_400,
    min_offer_ttl: 3600,
    max_offer_ttl: 30 * 86_400,
    defaults: {
      ship_window: 3 * 86_400,
      confirm_window: 7 * 86_400,
      fee_payer: 0,
      public_offer_ttl: 30 * 86_400,
      private_offer_ttl: 86_400,
    },
    wallet_init_versions: chain.walletInitVersions.map(toView),
  };
}

async function reply(route: Route, status: number, body: unknown) {
  await route.fulfill({
    status,
    contentType: "application/json",
    body: status === 204 ? "" : JSON.stringify(body),
  });
}
