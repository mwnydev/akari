// 钱包 SDK（viem + ZeroDev）。体积大，只能经 ./load.ts 动态加载，不得被页面静态 import（plan 第 12 节）。
import { escrowAbi, escrowConfigAbi, mockUsdtAbi, type WalletInitVersion } from "@akari/contracts";
import { orderCost } from "@akari/fees";
import { PasskeyValidatorContractVersion, toPasskeyValidator } from "@zerodev/passkey-validator";
import { createKernelAccount, createKernelAccountClient, encodeCallDataEpV07 } from "@zerodev/sdk";
import { KERNEL_V3_3 } from "@zerodev/sdk/constants";
import {
  bytesToHex,
  createClient,
  createPublicClient,
  defineChain,
  encodeFunctionData,
  hashTypedData,
  hexToBigInt,
  http,
  isAddressEqual,
  isHex,
  keccak256,
  parseEventLogs,
  rpcSchema,
  stringToHex,
  zeroAddress,
  type Address,
  type Calls,
  type Hex,
  type Narrow,
} from "viem";
import { getEip712Domain } from "viem/actions";
import type { PasskeyView, WalletApi, WalletOverview } from "./api";
import { chain } from "./config";
import { humanizeError } from "./errors";
import { InitVersions } from "./init-version";
import { FEE_PAYERS, offerToJson, parseOffer, type Offer, type SignedOffer } from "./offer";
import { assertPasskey, ensureRecentPasskey, registerPasskey, toWebAuthnKey } from "./passkey";
import {
  assertSponsorFee,
  bundlerTransport,
  getSponsorClient,
  sponsorFeeFor,
  type PaymasterFields,
  type SponsoredAction,
} from "./sponsor";
import { OFFER_TYPES } from "./typed-data";

const viemChain = defineChain({
  id: chain.chainId,
  name: chain.name,
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: [chain.rpcUrl] } },
});
const publicClient = createPublicClient({ chain: viemChain, transport: http(chain.rpcUrl) });
const { MockUSDT: token, Escrow: escrow, EscrowConfig: escrowConfig } = chain.contracts;

/** 与 EscrowConfig.SCOPE_ESCROW 相同（一次性担保）。 */
const SCOPE_ESCROW = 1;

type GasPriceTier = { maxFeePerGas: Hex; maxPriorityFeePerGas: Hex };
const bundlerRpc = createClient({
  transport: bundlerTransport(),
  rpcSchema: rpcSchema<
    [
      {
        Method: "pimlico_getUserOperationGasPrice";
        Parameters: [];
        ReturnType: Record<"slow" | "standard" | "fast", GasPriceTier>;
      },
    ]
  >(),
});

/** 链配置里的版本，与后端 `wallet_init_versions` 核对后使用（见 ./init-version.ts 的信任边界）。 */
async function initVersions(api: WalletApi): Promise<InitVersions> {
  const params = await api.tradeParams();
  return InitVersions.resolve(chain.walletInitVersions, params.wallet_init_versions);
}

/**
 * 账户部署时（initCode 里的 Kernel initConfig）给该版本的 Paymaster 授权（plan 第 5 节、ADR 0004）：
 * 付款前操作的代付费在验证阶段扣，必须在 callData 之前就有授权。这是账户初始参数的一部分，改了地址就变。
 */
async function initConfig(v: WalletInitVersion): Promise<Hex[]> {
  const approve = encodeFunctionData({
    abi: mockUsdtAbi,
    functionName: "approve",
    args: [v.paymaster, BigInt(v.approveAmount)],
  });
  return [await encodeCallDataEpV07([{ to: v.approveToken, value: 0n, data: approve }])];
}

/** 按钱包自己的初始化参数版本组装 Kernel v3.3 账户（ADR 0008：绝不用「当前」配置推算老钱包）。 */
async function kernelAccount(passkey: PasskeyView, v: WalletInitVersion) {
  const entryPoint = { address: v.entryPoint, version: "0.7" } as const;
  const validator = await toPasskeyValidator(publicClient, {
    webAuthnKey: toWebAuthnKey(passkey),
    entryPoint,
    kernelVersion: KERNEL_V3_3,
    validatorContractVersion: PasskeyValidatorContractVersion.V0_0_3_PATCHED,
    validatorAddress: v.webauthnValidator,
  });
  return createKernelAccount(publicClient, {
    plugins: { sudo: validator },
    entryPoint,
    kernelVersion: KERNEL_V3_3,
    accountImplementationAddress: v.kernelImpl,
    factoryAddress: v.factory,
    metaFactoryAddress: v.metaFactory,
    index: 0n,
    // SDK 的类型只给 "0.3.1" 开放 initConfig，运行时 v3.1+ 都支持（Kernel v3.3 initialize 第 5 个参数）
    initConfig: (await initConfig(v)) as never,
  });
}

/** 该 Passkey 在该版本下的 counterfactual 地址（CREATE2，未部署也能算，不需要 Face ID）。 */
export async function walletAddress(passkey: PasskeyView, v: WalletInitVersion): Promise<Address> {
  return (await kernelAccount(passkey, v)).address;
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
async function quoteCheckout(offer: Offer): Promise<CheckoutQuote> {
  const q = await publicClient.readContract({
    address: escrowConfig,
    abi: escrowConfigAbi,
    functionName: "preview",
    args: [SCOPE_ESCROW],
  });
  const feePayer = FEE_PAYERS[offer.feePayer];
  if (!feePayer) throw new Error("报价里的手续费承担方无效");
  const c = orderCost({
    amount: offer.amount,
    feePayer,
    feeBps: q.feeBps,
    minFee: q.minFee,
    campaign: q.campaign,
    baseGas: q.baseGas,
    disputeMulBps: q.disputeMulBps,
  });
  const sponsorFee = sponsorFeeFor("createAndFund");
  return { buyerPays: c.buyerPays, sponsorFee, total: c.buyerPays + sponsorFee };
}

/**
 * 担保合约的 EIP-712 域，从合约读（ERC-5267）。viem 返回的 domain 总带着 salt，必须按 `fields` 位图取舍；
 * OpenZeppelin EIP712 固定是 0x0f（name、version、chainId、verifyingContract，无 salt）。
 */
async function offerDomain() {
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
  const domain = await offerDomain();
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
  const account = await kernelAccount(passkey, v);
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
      userOperation: {
        async estimateFeesPerGas() {
          const { standard } = await bundlerRpc.request({
            method: "pimlico_getUserOperationGasPrice",
            params: [],
          });
          return {
            maxFeePerGas: hexToBigInt(standard.maxFeePerGas),
            maxPriorityFeePerGas: hexToBigInt(standard.maxPriorityFeePerGas),
          };
        },
      },
    });
    try {
      const hash = await client.sendUserOperation({ calls });
      const receipt = await client.waitForUserOperationReceipt({ hash });
      if (!receipt.success) {
        throw new Error(
          expectedFee > 0n
            ? "链上执行失败（代付费已扣除），请刷新后查看订单状态"
            : "链上执行失败，请刷新后查看订单状态",
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
          domain: await offerDomain(),
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
  };
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
  const [quote, balance] = await Promise.all([quoteCheckout(offer), wallet.balance()]);
  return { signed, quote, balance };
}
