// 钱包 SDK（viem + ZeroDev）。体积大，只能经 ./load.ts 动态加载，不得被页面静态 import（plan 第 12 节）。
import {
  escrowAbi,
  escrowConfigAbi,
  mockUsdtAbi,
  streamEscrowAbi,
  type WalletInitVersion,
} from "@akari/contracts";
import { orderCost } from "@akari/fees";
import { createKernelAccountClient } from "@zerodev/sdk";
import {
  bytesToHex,
  createPublicClient,
  defineChain,
  hashTypedData,
  hexToBigInt,
  http,
  isAddressEqual,
  isHex,
  keccak256,
  parseEventLogs,
  stringToHex,
  zeroAddress,
  type Address,
  type Calls,
  type Hex,
  type Narrow,
} from "viem";
import { getEip712Domain } from "viem/actions";
import type { AuctionView, PasskeyView, WalletApi, WalletOverview } from "./api";
import {
  CURRENCY_CNY,
  auctionOrderAmount,
  checkAuthorization,
  parseAuctionAuth,
  parseSettlementProof,
  type AuctionAuth,
  type SettlementProof,
} from "./auction";
import { READ_RPC_URL, chain } from "./config";
import { humanizeError } from "./errors";
import { InitVersions } from "./init-version";
import { bundlerGasPrice, kernelAccount } from "./kernel";
import { FEE_PAYERS, offerToJson, parseOffer, type Offer, type SignedOffer } from "./offer";
import { assertPasskey, ensureRecentPasskey, registerPasskey, toWebAuthnKey } from "./passkey";
import {
  assertSponsorFee,
  bundlerTransport,
  getSponsorClient,
  type PaymasterFields,
} from "./sponsor";
import { sponsorFeeFor, type SponsoredAction } from "./sponsor-fee";
import { AUCTION_TYPES, OFFER_TYPES } from "./typed-data";

const viemChain = defineChain({
  id: chain.chainId,
  name: chain.name,
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: [READ_RPC_URL] } },
});
// forum-api 的只读代理按会话限流，要带论坛会话 Cookie（ADR 0009）；本地 anvil 不能带（它的 CORS 是 `*`）
const publicClient = createPublicClient({
  chain: viemChain,
  transport:
    READ_RPC_URL === chain.rpcUrl
      ? http(READ_RPC_URL)
      : http(READ_RPC_URL, { fetchOptions: { credentials: "include" } }),
});
const {
  MockUSDT: token,
  Escrow: escrow,
  EscrowConfig: escrowConfig,
  StreamEscrow: streamEscrow,
} = chain.contracts;

/** 与 EscrowConfig.SCOPE_ESCROW / SCOPE_AUCTION 相同（一次性担保 / 拍卖成交；免费活动按范围生效）。 */
const SCOPE_ESCROW = 1;
const SCOPE_AUCTION = 2;

const gasPrice = bundlerGasPrice(bundlerTransport());

/** 链配置里的版本，与后端 `wallet_init_versions` 核对后使用（见 ./init-version.ts 的信任边界）。 */
async function initVersions(api: WalletApi): Promise<InitVersions> {
  const params = await api.tradeParams();
  return InitVersions.resolve(chain.walletInitVersions, params.wallet_init_versions);
}

/** 该 Passkey 在该版本下的 counterfactual 地址（CREATE2，未部署也能算，不需要 Face ID）。 */
export async function walletAddress(passkey: PasskeyView, v: WalletInitVersion): Promise<Address> {
  return (await kernelAccount(publicClient, toWebAuthnKey(passkey), v)).address;
}

export interface CheckoutQuote {
  /** 本次 approve 给担保合约的金额 = 货款 + 买家承担的手续费与 gas 费 */
  buyerPays: bigint;
  /** Paymaster 在验证阶段扣的代付费 */
  sponsorFee: bigint;
  /** 钱包实际转出 = buyerPays + sponsorFee */
  total: bigint;
}

/** 按链上当前参数（EscrowConfig.preview）算这单买家要付多少；后端的估算只作参考，以链上为准。 */
async function quoteCheckout(
  amount: bigint,
  feePayerCode: number,
  scope: number,
  action: SponsoredAction,
): Promise<CheckoutQuote> {
  const q = await publicClient.readContract({
    address: escrowConfig,
    abi: escrowConfigAbi,
    functionName: "preview",
    args: [scope],
  });
  const feePayer = FEE_PAYERS[feePayerCode];
  if (!feePayer) throw new Error("手续费承担方无效");
  const c = orderCost({
    amount,
    feePayer,
    feeBps: q.feeBps,
    minFee: q.minFee,
    campaign: q.campaign,
    baseGas: q.baseGas,
    disputeMulBps: q.disputeMulBps,
  });
  const sponsorFee = sponsorFeeFor(action);
  return { buyerPays: c.buyerPays, sponsorFee, total: c.buyerPays + sponsorFee };
}

/**
 * 担保合约的 EIP-712 域，从合约读（ERC-5267）。viem 返回的 domain 总带着 salt，必须按 `fields` 位图取舍；
 * OpenZeppelin EIP712 固定是 0x0f（name、version、chainId、verifyingContract，无 salt）。
 */
async function escrowDomain() {
  const { domain, fields } = await getEip712Domain(publicClient, { address: escrow });
  if (fields !== "0x0f") throw new Error("担保合约的签名域格式不支持");
  const { name, version, chainId, verifyingContract } = domain;
  return { name, version, chainId, verifyingContract };
}

/**
 * 验证卖家签名：先确认本地算的 EIP-712 哈希与合约 `hashOffer` 一致（类型定义没漂移），
 * 再用 viem verifyTypedData（eth_call，含 ERC-6492：卖家钱包还没部署也能验）。
 */
async function verifyOffer({ offer, signature }: SignedOffer): Promise<boolean> {
  const domain = await escrowDomain();
  const typed = { domain, types: OFFER_TYPES, primaryType: "Offer", message: offer } as const;
  const onchain = await publicClient.readContract({
    address: escrow,
    abi: escrowAbi,
    functionName: "hashOffer",
    args: [offer],
  });
  if (hashTypedData(typed) !== onchain) throw new Error("报价签名格式与担保合约不一致");
  return publicClient.verifyTypedData({ ...typed, address: offer.seller, signature });
}

/** 拍卖授权的 typed data；先确认本地哈希与合约 `hashAuction` 一致（类型定义没漂移）。 */
async function auctionTypedData(auth: AuctionAuth) {
  const typed = {
    domain: await escrowDomain(),
    types: AUCTION_TYPES,
    primaryType: "AuctionAuth",
    message: auth,
  } as const;
  const onchain = await publicClient.readContract({
    address: escrow,
    abi: escrowAbi,
    functionName: "hashAuction",
    args: [auth],
  });
  if (hashTypedData(typed) !== onchain) throw new Error("拍卖授权的签名格式与担保合约不一致");
  return { typed, hash: onchain };
}

/** 卖家签的拍卖授权（含 ERC-6492：卖家钱包还没部署也能验）。 */
async function verifyAuction(auth: AuctionAuth, signature: Hex): Promise<boolean> {
  const { typed } = await auctionTypedData(auth);
  return publicClient.verifyTypedData({ ...typed, address: auth.seller, signature });
}

async function readBalance(address: Address): Promise<bigint> {
  return publicClient.readContract({
    address: token,
    abi: mockUsdtAbi,
    functionName: "balanceOf",
    args: [address],
  });
}

/** 报价条款（不含由钱包与链上参数决定的字段）。 */
export interface OfferDraft {
  /** 只给这位买家（私信里的报价）；省略 = 公开报价 */
  buyer?: Address | undefined;
  amount: bigint;
  /** 交易帖的内容哈希；私信报价没有交易帖，约定标题的 keccak256 写进 listingHash，仲裁时据此对照聊天记录 */
  subject: { listingHash: Hex } | { title: string };
  feePayer: number;
  bond: bigint;
  lateSlashBps: number;
  instant: boolean;
  shipWindow: number;
  confirmWindow: number;
  /** 有效期（秒），在后端 `min_offer_ttl`–`max_offer_ttl` 之间 */
  ttl: number;
}

/** 报价 nonce：随机 64 位，卖家可凭它 `cancelOffer` 撤销这份报价。 */
function randomNonce(): bigint {
  return hexToBigInt(bytesToHex(crypto.getRandomValues(new Uint8Array(8))));
}

async function openKernel(passkey: PasskeyView, v: WalletInitVersion, versions: InitVersions) {
  const account = await kernelAccount(publicClient, toWebAuthnKey(passkey), v);
  const paymasters = versions.paymasters();

  // 调用带 abi，bundler 估算失败时 viem 能把合约 revert 解码成错误名（errors.ts 再翻译成人话）
  async function send<const calls extends readonly unknown[]>(
    action: SponsoredAction,
    calls: Calls<Narrow<calls>>,
  ) {
    const sponsor = await getSponsorClient();
    const expectedFee = sponsorFeeFor(action);
    const checked = (fields: PaymasterFields) => {
      assertSponsorFee(fields, expectedFee, paymasters);
      return fields;
    };
    const client = createKernelAccountClient({
      account,
      chain: viemChain,
      client: publicClient,
      bundlerTransport: bundlerTransport(),
      paymaster: {
        getPaymasterStubData: async (op) => checked(await sponsor.getPaymasterStubData(action, op)),
        getPaymasterData: async (op) => checked(await sponsor.getPaymasterData(action, op)),
      },
      userOperation: { estimateFeesPerGas: gasPrice },
    });
    try {
      const hash = await client.sendUserOperation({ calls });
      const receipt = await client.waitForUserOperationReceipt({ hash });
      if (!receipt.success) {
        const check = action === "transfer" ? "余额" : "订单状态";
        throw new Error(
          expectedFee > 0n
            ? `链上执行失败（代付费已扣除），请刷新后查看${check}`
            : `链上执行失败，请刷新后查看${check}`,
        );
      }
      return receipt;
    } catch (e) {
      throw humanizeError(e);
    }
  }

  return {
    address: account.address,
    initVersion: v.version,

    balance: () => readBalance(account.address),

    /** 卖家签报价（钱包未部署时 viem 自动包成 ERC-6492 签名）。费率上限按链上当前参数填。 */
    async signOffer(draft: OfferDraft): Promise<SignedOffer> {
      const params = await publicClient.readContract({
        address: escrowConfig,
        abi: escrowConfigAbi,
        functionName: "params",
      });
      const offer: Offer = {
        seller: account.address,
        buyer: draft.buyer ?? zeroAddress,
        token,
        amount: draft.amount,
        feePayer: draft.feePayer,
        maxFeeBps: Number(params.feeBps),
        maxBaseGas: params.baseGas,
        maxMinFee: params.minFee,
        bond: draft.bond,
        lateSlashBps: draft.lateSlashBps,
        instant: draft.instant,
        shipWindow: draft.shipWindow,
        confirmWindow: draft.confirmWindow,
        listingHash:
          "listingHash" in draft.subject
            ? draft.subject.listingHash
            : keccak256(stringToHex(draft.subject.title)),
        expiry: BigInt(Math.floor(Date.now() / 1000) + draft.ttl),
        maxFills: 1,
        nonce: randomNonce(),
      };
      try {
        const signature = await account.signTypedData({
          domain: await escrowDomain(),
          types: OFFER_TYPES,
          primaryType: "Offer",
          message: offer,
        });
        return { offer, signature };
      } catch (e) {
        throw humanizeError(e);
      }
    },

    /** 买家：approve + createAndFund 合成一个 UserOperation，一次 Face ID。返回 Escrow 上的链上订单号。 */
    async pay({ offer, signature }: SignedOffer, quote: CheckoutQuote) {
      const receipt = await send("createAndFund", [
        { to: token, abi: mockUsdtAbi, functionName: "approve", args: [escrow, quote.buyerPays] },
        { to: escrow, abi: escrowAbi, functionName: "createAndFund", args: [offer, signature] },
      ]);
      const [created] = parseEventLogs({
        abi: escrowAbi,
        logs: receipt.logs,
        eventName: "OrderCreated",
      });
      if (!created) throw new Error("付款已上链，但没有找到订单号，请刷新订单列表");
      return { orderId: created.args.orderId, txHash: receipt.receipt.transactionHash };
    },

    /** 卖家签拍卖授权（钱包未部署时 viem 自动包成 ERC-6492 签名）。 */
    async signAuction(auth: AuctionAuth): Promise<Hex> {
      try {
        const { typed } = await auctionTypedData(auth);
        return await account.signTypedData(typed);
      } catch (e) {
        throw humanizeError(e);
      }
    },

    /** 拍卖赢家：approve + createFromAuction 合成一个 UserOperation，一次 Face ID。返回链上订单号。 */
    async payAuction(c: AuctionCheckout) {
      const receipt = await send("createFromAuction", [
        { to: token, abi: mockUsdtAbi, functionName: "approve", args: [escrow, c.quote.buyerPays] },
        {
          to: escrow,
          abi: escrowAbi,
          functionName: "createFromAuction",
          args: [c.auth, c.sellerSig, c.proof, c.platformSig],
        },
      ]);
      const [created] = parseEventLogs({
        abi: escrowAbi,
        logs: receipt.logs,
        eventName: "OrderCreated",
      });
      if (!created) throw new Error("付款已上链，但没有找到订单号，请刷新订单列表");
      return { orderId: created.args.orderId, txHash: receipt.receipt.transactionHash };
    },

    async markDelivered(orderId: bigint) {
      const r = await send("markDelivered", [
        { to: escrow, abi: escrowAbi, functionName: "markDelivered", args: [orderId] },
      ]);
      return r.receipt.transactionHash;
    },

    async release(orderId: bigint) {
      const r = await send("release", [
        { to: escrow, abi: escrowAbi, functionName: "release", args: [orderId] },
      ]);
      return r.receipt.transactionHash;
    },

    /**
     * 提回交易所：USDT `transfer(to, amount)`，代付费另由 Paymaster 在验证阶段扣（钱包共转出 amount + 代付费）。
     * forum-sponsor 只代付转到本人已生效提币地址的转账。先按链上余额核对，不够就不弹 Face ID。
     */
    async withdraw(to: Address, amount: bigint) {
      const fee = sponsorFeeFor("transfer");
      if ((await readBalance(account.address)) < amount + fee) {
        throw new Error("余额不足（还要留出代付费），请刷新后重试");
      }
      const r = await send("transfer", [
        { to: token, abi: mockUsdtAbi, functionName: "transfer", args: [to, amount] },
      ]);
      return r.receipt.transactionHash;
    },

    /** 分期订单卖家：开始服务，解锁从现在开始计时。 */
    async start(orderId: bigint) {
      const r = await send("start", [
        { to: streamEscrow, abi: streamEscrowAbi, functionName: "start", args: [orderId] },
      ]);
      return r.receipt.transactionHash;
    },

    /** 分期订单卖家：领取已解锁部分（当次扣领取 gas 费；到期后领取即结清）。 */
    async claim(orderId: bigint) {
      const r = await send("claim", [
        { to: streamEscrow, abi: streamEscrowAbi, functionName: "claim", args: [orderId] },
      ]);
      return r.receipt.transactionHash;
    },
  };
}

/** 分期订单的链上状态（latest）：净本金、已领取、上次领取时间、领取 gas 费等 indexer 不提供的字段。 */
export async function streamOrder(orderId: bigint) {
  return publicClient.readContract({
    address: streamEscrow,
    abi: streamEscrowAbi,
    functionName: "getOrder",
    args: [orderId],
  });
}

/** 敏感操作（改提币地址）前的近期 Passkey 验证；已有就不再弹 Face ID。 */
export async function confirmWithPasskey(api: WalletApi): Promise<void> {
  try {
    await ensureRecentPasskey(api);
  } catch (e) {
    throw humanizeError(e);
  }
}

export type Wallet = Awaited<ReturnType<typeof openKernel>>;

/**
 * 已开通的用户：用登记的 Passkey 公钥、按钱包登记时的版本算出同一个智能账户（不需要 Face ID，不上链）。
 * 算出的地址必须与后端登记的一致，否则宁可报错也不显示一个可能收不到钱的地址。
 */
export async function openWallet(api: WalletApi, overview: WalletOverview): Promise<Wallet> {
  const w = overview.wallets.find((x) => x.chain_id === chain.chainId);
  const passkey = w && overview.passkeys.find((p) => p.id === w.passkey_id);
  if (!w || !passkey) throw new Error("没有找到本网络的钱包，请重新开通交易");
  const versions = await initVersions(api);
  const wallet = await openKernel(passkey, versions.get(w.init_version), versions);
  if (!isHex(w.address) || !isAddressEqual(wallet.address, w.address)) {
    throw new Error("算出的钱包地址与登记的不一致，已停止操作，请联系客服");
  }
  return wallet;
}

/** 按最新版本算出地址并登记到后端（需要会话里有刚验证过的这个 Passkey）。 */
async function bindWallet(api: WalletApi, passkey: PasskeyView): Promise<Wallet> {
  const versions = await initVersions(api);
  const v = versions.latest();
  const wallet = await openKernel(passkey, v, versions);
  await api.bindWallet({
    chain_id: chain.chainId,
    address: wallet.address,
    init_version: v.version,
  });
  return wallet;
}

/** 开通交易第 2–3 步：创建 Passkey（一次 Face ID，同时算作近期验证）→ 算出 Kernel 地址（未部署）→ 登记。 */
export async function createWallet(api: WalletApi): Promise<Wallet> {
  try {
    return await bindWallet(api, await registerPasskey(api));
  } catch (e) {
    throw humanizeError(e);
  }
}

/** Passkey 已注册但地址没登记上（例如上次开通时断网）：再验证一次 Passkey，补登记地址。 */
export async function bindRegisteredPasskey(
  api: WalletApi,
  overview: WalletOverview,
): Promise<Wallet> {
  try {
    const { passkey_id } = await assertPasskey(api);
    const passkey = overview.passkeys.find((p) => p.id === passkey_id);
    if (!passkey) throw new Error("没有找到这个 Passkey，请刷新后重试");
    return await bindWallet(api, passkey);
  } catch (e) {
    throw humanizeError(e);
  }
}

/** 报价卡片挂在交易帖下，或发在私信会话里（后端要求二选一）。 */
export type OfferTarget = { listing_id: number } | { conversation_id: number };

/**
 * 卖家发起担保：先确保近期 Passkey 验证（plan「私信发起担保」是敏感操作）→ Passkey 签 EIP-712 报价 →
 * 链上验签 → 提交报价卡片。返回卡片 ID（买家付款页 `/pay?offer=`）。
 */
export async function publishOffer(
  api: WalletApi,
  wallet: Wallet,
  draft: OfferDraft,
  target: OfferTarget,
): Promise<number> {
  try {
    await ensureRecentPasskey(api);
    const signed = await wallet.signOffer(draft);
    if (!(await verifyOffer(signed))) throw new Error("签名没有通过链上验证，请重试");
    const card = await api.createOffer({
      ...target,
      offer: offerToJson(signed.offer),
      signature: signed.signature,
    });
    return card.id;
  } catch (e) {
    throw humanizeError(e);
  }
}

export interface Checkout {
  signed: SignedOffer;
  quote: CheckoutQuote;
  balance: bigint;
}

/**
 * 买家付款页：向后端接受卡片拿付款数据，再逐项核对——网络与担保合约、付款钱包、指定买家、
 * 卖家签名（链上验证）——都对得上才给出报价（链上参数算的费用）与余额。
 */
export async function prepareCheckout(
  api: WalletApi,
  wallet: Wallet,
  offerId: number,
): Promise<Checkout> {
  const payment = await api.acceptOffer(offerId);
  if (
    payment.chain_id !== chain.chainId ||
    !isHex(payment.escrow) ||
    !isAddressEqual(payment.escrow, escrow)
  ) {
    throw new Error("这份报价不属于当前网络的担保合约，请不要付款");
  }
  if (!isHex(payment.buyer_wallet) || !isAddressEqual(payment.buyer_wallet, wallet.address)) {
    throw new Error("付款钱包与你登记的钱包不一致，请刷新后重试");
  }
  const offer = parseOffer(payment.offer);
  if (!offer || !isHex(payment.signature)) throw new Error("报价数据不完整，请让卖家重新发送");
  if (!isAddressEqual(offer.buyer, zeroAddress) && !isAddressEqual(offer.buyer, wallet.address)) {
    throw new Error("这份报价指定了别的买家");
  }
  const signed = { offer, signature: payment.signature };
  if (!(await verifyOffer(signed))) throw new Error("卖家签名无效，请不要付款");
  const [quote, balance] = await Promise.all([
    quoteCheckout(offer.amount, offer.feePayer, SCOPE_ESCROW, "createAndFund"),
    wallet.balance(),
  ]);
  return { signed, quote, balance };
}

/**
 * 费率上限必须等于链上当前参数（与签报价相同）：低了成交时合约拒绝建单，高了等于预先同意将来涨价。
 */
async function checkFeeCaps(auth: AuctionAuth): Promise<void> {
  const params = await publicClient.readContract({
    address: escrowConfig,
    abi: escrowConfigAbi,
    functionName: "params",
  });
  if (
    auth.maxFeeBps !== params.feeBps ||
    auth.maxBaseGas !== params.baseGas ||
    auth.maxMinFee !== params.minFee
  ) {
    throw new Error("拍卖授权里的费率上限与链上当前参数不一致，请回到论坛重新发起");
  }
}

export interface AuctionReview {
  /** 将要签名的授权（确认页显示的就是它） */
  auth: AuctionAuth;
  /** 卖家在论坛填的拍卖（起拍价、加价幅度、时长等不进签名的条件） */
  view: AuctionView;
  cny: boolean;
  /** 保留价折合的 USDT（与合约换算一致） */
  reserveUsdt: bigint;
  /** 这个时间之前提交签名，否则重新开始 */
  signDeadline: string;
}

/**
 * 卖家挂拍第二步：近期 Passkey 验证 → 后端生成拍卖授权 → 逐项核对（严格解析、网络与担保合约、卖家是本钱包、
 * 币种合约、编号、期限、有效期、与论坛拍卖条件一致、费率上限 = 链上当前值、本地算的授权哈希 = 后端给的 = 合约
 * `hashAuction`），不对就抛出原因。返回确认页显示的、之后签名的同一个对象。
 */
export async function startAuctionSigning(
  api: WalletApi,
  wallet: Wallet,
  auctionId: number,
): Promise<AuctionReview> {
  try {
    await ensureRecentPasskey(api);
    const view = await api.auction(auctionId);
    const signing = await api.startAuction(auctionId);
    const auth = checkAuthorization(signing, view, wallet.address);
    await checkFeeCaps(auth);
    const { hash } = await auctionTypedData(auth);
    if (!isHex(signing.auth_hash) || hash.toLowerCase() !== signing.auth_hash.toLowerCase()) {
      throw new Error("后端给出的授权哈希与本地计算的不一致，请不要签名");
    }
    return {
      auth,
      view,
      cny: auth.currency === CURRENCY_CNY,
      reserveUsdt: auctionOrderAmount(auth, auth.reservePrice),
      signDeadline: signing.sign_deadline,
    };
  } catch (e) {
    throw humanizeError(e);
  }
}

/**
 * 卖家挂拍第三步：Passkey 签 EIP-712 拍卖授权 → 链上验签 → 提交给后端（后端同样验签），发拍卖帖、开始计时。
 * 返回上线后的拍卖（带拍卖帖主题）。
 */
export async function publishAuctionAuth(
  api: WalletApi,
  wallet: Wallet,
  review: AuctionReview,
): Promise<AuctionView> {
  try {
    if (Date.parse(review.signDeadline) <= Date.now()) {
      throw new Error("签名已超时，请刷新本页重新生成授权");
    }
    const signature = await wallet.signAuction(review.auth);
    if (!(await verifyAuction(review.auth, signature))) {
      throw new Error("签名没有通过链上验证，请重试");
    }
    return await api.finishAuction(review.view.id, signature);
  } catch (e) {
    throw humanizeError(e);
  }
}

export interface AuctionCheckout {
  auth: AuctionAuth;
  cny: boolean;
  sellerSig: Hex;
  proof: SettlementProof;
  platformSig: Hex;
  /** 合约建单的 USDT 金额（成交价按卖家汇率换算后） */
  amount: bigint;
  quote: CheckoutQuote;
  balance: bigint;
}

/** 平台成交证明的签名者必须是链上登记的 `auctionSigner`（与合约同样的校验，付款前就挡掉无效证明）。 */
async function verifyProof(proof: SettlementProof, signature: Hex): Promise<boolean> {
  const [signer, hash] = await Promise.all([
    publicClient.readContract({
      address: escrowConfig,
      abi: escrowConfigAbi,
      functionName: "auctionSigner",
    }),
    publicClient.readContract({
      address: escrow,
      abi: escrowAbi,
      functionName: "hashProof",
      args: [proof],
    }),
  ]);
  if (isAddressEqual(signer, zeroAddress)) throw new Error("拍卖成交尚未在链上开通");
  return publicClient.verifyHash({ address: signer, hash, signature });
}

/**
 * 拍卖赢家付款页：向后端取付款数据，再逐项核对——网络与担保合约、付款人就是本钱包、授权哈希与成交证明一致、
 * 卖家授权与平台证明的签名（链上验证）、成交价不低于保留价、未过期——并自己按卖家汇率算出建单金额，
 * 与后端给的不一致就拒绝。全部对得上才给出费用（链上参数）与余额。
 */
export async function prepareAuctionCheckout(
  api: WalletApi,
  wallet: Wallet,
  auctionId: number,
): Promise<AuctionCheckout> {
  const p = await api.auctionPayment(auctionId);
  if (p.chain_id !== chain.chainId || !isHex(p.escrow) || !isAddressEqual(p.escrow, escrow)) {
    throw new Error("这场拍卖不属于当前网络的担保合约，请不要付款");
  }
  const auth = parseAuctionAuth(p.auth);
  const proof = parseSettlementProof(p.proof);
  if (!auth || !proof || !isHex(p.seller_signature) || !isHex(p.platform_signature)) {
    throw new Error("成交数据不完整，请稍后刷新");
  }
  if (
    p.auction_id !== auctionId ||
    auth.auctionId !== BigInt(auctionId) ||
    !isAddressEqual(auth.token, token) ||
    p.price !== proof.price.toString()
  ) {
    throw new Error("成交数据与这场拍卖不符，请不要付款");
  }
  if (
    !isAddressEqual(proof.winner, wallet.address) ||
    !isHex(p.buyer_wallet) ||
    !isAddressEqual(p.buyer_wallet, wallet.address)
  ) {
    throw new Error("成交证明里的赢家不是你的钱包");
  }
  const now = BigInt(Math.floor(Date.now() / 1000));
  if (proof.payDeadline <= now || auth.expiry <= now) throw new Error("付款期限已过");
  if (proof.price < auth.reservePrice) throw new Error("成交价低于卖家的保留价，请不要付款");
  const { hash } = await auctionTypedData(auth);
  if (proof.authHash !== hash) throw new Error("成交证明与卖家授权不对应，请不要付款");
  const [sellerOk, proofOk] = await Promise.all([
    verifyAuction(auth, p.seller_signature),
    verifyProof(proof, p.platform_signature),
  ]);
  if (!sellerOk) throw new Error("卖家授权签名无效，请不要付款");
  if (!proofOk) throw new Error("平台成交证明无效，请不要付款");
  const amount = auctionOrderAmount(auth, proof.price);
  if (p.amount !== amount.toString()) {
    throw new Error("后端给出的应付金额与按卖家汇率换算的结果不一致，请不要付款");
  }
  const [quote, balance] = await Promise.all([
    quoteCheckout(amount, auth.feePayer, SCOPE_AUCTION, "createFromAuction"),
    wallet.balance(),
  ]);
  return {
    auth,
    cny: auth.currency === CURRENCY_CNY,
    sellerSig: p.seller_signature,
    proof,
    platformSig: p.platform_signature,
    amount,
    quote,
    balance,
  };
}
