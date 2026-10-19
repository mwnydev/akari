// 钱包 SDK（viem + ZeroDev）。体积大，只能经 ./load.ts 动态加载，不得被页面静态 import（plan 第 12 节）。
import { escrowAbi, escrowConfigAbi, mockUsdtAbi } from "@akari/contracts";
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
  keccak256,
  maxUint256,
  parseEventLogs,
  rpcSchema,
  stringToHex,
  zeroAddress,
  type Address,
  type Calls,
  type Hex,
  type Narrow,
} from "viem";
import type { GetPaymasterDataParameters } from "viem/account-abstraction";
import { getEip712Domain } from "viem/actions";
import type { StoredPasskey, WalletApi } from "./api";
import { chain } from "./config";
import { humanizeError } from "./errors";
import { FEE_PAYERS, type Offer, type SignedOffer } from "./offer";
import { registerPasskey, toWebAuthnKey } from "./passkey";
import {
  assertSponsorFee,
  getSponsorClient,
  type SponsorableUserOperation,
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
const entryPoint = { address: chain.entryPoint, version: "0.7" } as const;
const { MockUSDT: token, Escrow: escrow, EscrowConfig: escrowConfig } = chain.contracts;

/** 与 EscrowConfig.SCOPE_ESCROW 相同（一次性担保）。 */
const SCOPE_ESCROW = 1;

type GasPriceTier = { maxFeePerGas: Hex; maxPriorityFeePerGas: Hex };
const bundlerRpc = createClient({
  transport: http(chain.bundlerUrl),
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

/**
 * 账户部署时（initCode 里的 Kernel initConfig）给 Paymaster 授权 USDT（plan 第 5 节、ADR 0004）：
 * 付款前操作的代付费在验证阶段扣，必须在 callData 之前就有授权。这是账户初始参数的一部分，改了地址就变。
 */
async function initConfig(): Promise<Hex[]> {
  const approve = encodeFunctionData({
    abi: mockUsdtAbi,
    functionName: "approve",
    args: [chain.contracts.FixedFeePaymaster, maxUint256],
  });
  return [await encodeCallDataEpV07([{ to: token, value: 0n, data: approve }])];
}

async function kernelAccount(passkey: StoredPasskey) {
  const validator = await toPasskeyValidator(publicClient, {
    webAuthnKey: await toWebAuthnKey(passkey),
    entryPoint,
    kernelVersion: KERNEL_V3_3,
    validatorContractVersion: PasskeyValidatorContractVersion.V0_0_3_PATCHED,
    validatorAddress: chain.webAuthnValidator,
  });
  return createKernelAccount(publicClient, {
    plugins: { sudo: validator },
    entryPoint,
    kernelVersion: KERNEL_V3_3,
    accountImplementationAddress: chain.kernel.implementation,
    factoryAddress: chain.kernel.factory,
    metaFactoryAddress: chain.kernel.metaFactory,
    index: 0n,
    // SDK 的类型只给 "0.3.1" 开放 initConfig，运行时 v3.1+ 都支持（Kernel v3.3 initialize 第 5 个参数）
    initConfig: (await initConfig()) as never,
  });
}

/** viem 在 getPaymasterData 时已经填好全部 gas 字段；缺字段说明调用顺序不对，宁可失败也不签不完整的操作。 */
function sponsorable(op: GetPaymasterDataParameters): SponsorableUserOperation {
  if ("initCode" in op) throw new Error("只支持 EntryPoint v0.7 的 UserOperation");
  const {
    callGasLimit,
    verificationGasLimit,
    preVerificationGas,
    maxFeePerGas,
    maxPriorityFeePerGas,
  } = op;
  if (
    callGasLimit === undefined ||
    verificationGasLimit === undefined ||
    preVerificationGas === undefined ||
    maxFeePerGas === undefined ||
    maxPriorityFeePerGas === undefined
  ) {
    throw new Error("UserOperation 的 gas 字段不完整");
  }
  return {
    sender: op.sender,
    nonce: op.nonce,
    callData: op.callData,
    factory: "factory" in op ? op.factory : undefined,
    factoryData: "factoryData" in op ? op.factoryData : undefined,
    callGasLimit,
    verificationGasLimit,
    preVerificationGas,
    maxFeePerGas,
    maxPriorityFeePerGas,
  };
}

export interface CheckoutQuote {
  /** 本次 approve 给担保合约的金额 = 货款 + 买家承担的手续费与 gas 费 */
  buyerPays: bigint;
  /** Paymaster 在验证阶段扣的代付费 */
  sponsorFee: bigint;
  /** 钱包实际转出 = buyerPays + sponsorFee */
  total: bigint;
}

/** 按链上当前参数（EscrowConfig.preview）算这单买家要付多少。 */
export async function quoteCheckout(offer: Offer): Promise<CheckoutQuote> {
  const [q, sponsorFee] = await Promise.all([
    publicClient.readContract({
      address: escrowConfig,
      abi: escrowConfigAbi,
      functionName: "preview",
      args: [SCOPE_ESCROW],
    }),
    getSponsorClient().then((s) => s.fee("createAndFund")),
  ]);
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
  return {
    buyerPays: c.buyerPays,
    sponsorFee,
    total: c.buyerPays + sponsorFee,
  };
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
export async function verifyOffer({ offer, signature }: SignedOffer): Promise<boolean> {
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

export type OrderView = Awaited<ReturnType<typeof readOrder>>;

export async function readOrder(orderId: bigint) {
  return publicClient.readContract({
    address: escrow,
    abi: escrowAbi,
    functionName: "getOrder",
    args: [orderId],
  });
}

export async function readBalance(address: Address): Promise<bigint> {
  return publicClient.readContract({
    address: token,
    abi: mockUsdtAbi,
    functionName: "balanceOf",
    args: [address],
  });
}

export interface OfferDraft {
  /** 只给这位买家（私信里的报价）；省略 = 公开报价 */
  buyer?: Address | undefined;
  amount: bigint;
  title: string;
  shipDays: number;
  confirmDays: number;
  validHours: number;
}

const DAY = 86_400;

/** 报价 nonce：随机 64 位，卖家可凭它 `cancelOffer` 撤销这份报价。 */
function randomNonce(): bigint {
  return hexToBigInt(bytesToHex(crypto.getRandomValues(new Uint8Array(8))));
}

async function openKernel(passkey: StoredPasskey) {
  const account = await kernelAccount(passkey);

  // 调用带 abi，bundler 估算失败时 viem 能把合约 revert 解码成错误名（errors.ts 再翻译成人话）
  async function send<const calls extends readonly unknown[]>(
    action: SponsoredAction,
    calls: Calls<Narrow<calls>>,
    expectedFee: bigint,
  ) {
    const sponsor = await getSponsorClient();
    const client = createKernelAccountClient({
      account,
      chain: viemChain,
      client: publicClient,
      bundlerTransport: http(chain.bundlerUrl),
      paymaster: {
        async getPaymasterStubData() {
          const fields = await sponsor.stub(action);
          assertSponsorFee(fields, expectedFee);
          return fields;
        },
        async getPaymasterData(op) {
          const fields = await sponsor.sponsor({ action, userOperation: sponsorable(op) });
          assertSponsorFee(fields, expectedFee);
          return fields;
        },
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

    balance: () => readBalance(account.address),

    /** 卖家签报价（钱包未部署时 viem 自动包成 ERC-6492 签名）。条款上限按链上当前参数填。 */
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
        feePayer: 0, // 卖家承担手续费（发帖默认，plan 第 6 节）
        maxFeeBps: Number(params.feeBps),
        maxBaseGas: params.baseGas,
        maxMinFee: params.minFee,
        bond: 0n,
        lateSlashBps: 0,
        instant: false,
        shipWindow: draft.shipDays * DAY,
        confirmWindow: draft.confirmDays * DAY,
        listingHash: keccak256(stringToHex(draft.title)),
        expiry: BigInt(Math.floor(Date.now() / 1000) + draft.validHours * 3600),
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

    /** 买家：approve + createAndFund 合成一个 UserOperation，一次 Face ID。返回新订单号。 */
    async pay({ offer, signature }: SignedOffer, quote: CheckoutQuote) {
      const receipt = await send(
        "createAndFund",
        [
          { to: token, abi: mockUsdtAbi, functionName: "approve", args: [escrow, quote.buyerPays] },
          { to: escrow, abi: escrowAbi, functionName: "createAndFund", args: [offer, signature] },
        ],
        quote.sponsorFee,
      );
      const [created] = parseEventLogs({
        abi: escrowAbi,
        logs: receipt.logs,
        eventName: "OrderCreated",
      });
      if (!created) throw new Error("付款已上链，但没有找到订单号，请刷新订单列表");
      return { orderId: created.args.orderId, txHash: receipt.receipt.transactionHash };
    },

    async markDelivered(orderId: bigint) {
      const r = await send(
        "markDelivered",
        [{ to: escrow, abi: escrowAbi, functionName: "markDelivered", args: [orderId] }],
        0n,
      );
      return r.receipt.transactionHash;
    },

    async release(orderId: bigint) {
      const r = await send(
        "release",
        [{ to: escrow, abi: escrowAbi, functionName: "release", args: [orderId] }],
        0n,
      );
      return r.receipt.transactionHash;
    },
  };
}

export type Wallet = Awaited<ReturnType<typeof openKernel>>;

/** 已开通的用户：用登记的 Passkey 公钥算出同一个智能账户（不需要 Face ID，不上链）。 */
export async function openWallet(api: WalletApi): Promise<Wallet | null> {
  const passkey = await api.getPasskey();
  return passkey ? openKernel(passkey) : null;
}

/** 开通交易：创建 Passkey → 算出 Kernel v3.3 账户地址（CREATE2，未部署）→ 写回后端。 */
export async function createWallet(api: WalletApi): Promise<Wallet> {
  try {
    const wallet = await openKernel(await registerPasskey(api));
    await api.saveWalletAddress(wallet.address);
    return wallet;
  } catch (e) {
    throw humanizeError(e);
  }
}
