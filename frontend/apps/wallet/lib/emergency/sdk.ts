// 紧急取回页的链上部分（ADR 0007）：完全不依赖论坛后端——不加载 api-client，不用平台代付。
// 读链走用户选的公共 RPC，UserOperation 不带 Paymaster、提交给用户填的第三方 Bundler，gas 由钱包里的 ETH 支付。
// 体积大（viem + ZeroDev），只能经 ./load.ts 动态加载。
import { escrowAbi, mockUsdtAbi, streamEscrowAbi, type WalletInitVersion } from "@akari/contracts";
import {
  createPublicClient,
  defineChain,
  http,
  isAddressEqual,
  parseEther,
  type Abi,
  type Address,
  type PublicClient,
} from "viem";
import { createBundlerClient, type BundlerClient } from "viem/account-abstraction";
import { chain } from "../wallet/config";
import { humanizeError } from "../wallet/errors";
import {
  bundlerGasPrice,
  kernelAccount,
  type FeesPerGas,
  type KernelAccount,
} from "../wallet/kernel";
import { getAssertion, passkeyKey, type PasskeyPublicKey } from "../wallet/webauthn";
import { recoverPasskeyKey } from "./recover";
import { requiredPrefund } from "./rules";

const { MockUSDT: token, Escrow: escrow, StreamEscrow: streamEscrow } = chain.contracts;

export type Market = "escrow" | "stream";
const MARKETS: Record<Market, Address> = { escrow, stream: streamEscrow };

export interface Connection {
  client: PublicClient;
  bundlerUrl: string;
}

/** 连上用户选的 RPC 与 Bundler，并核对链 ID 与 EntryPoint：连错网络宁可报错，也不在别的链上签名。 */
export async function connect(rpcUrl: string, bundlerUrl: string): Promise<Connection> {
  const viemChain = defineChain({
    id: chain.chainId,
    name: chain.name,
    nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
    rpcUrls: { default: { http: [rpcUrl] } },
  });
  const client = createPublicClient({ chain: viemChain, transport: http(rpcUrl) });
  let rpcChain: number;
  try {
    rpcChain = await client.getChainId();
  } catch {
    throw new Error("连不上这个 RPC，请换一个");
  }
  if (rpcChain !== chain.chainId) {
    throw new Error(`这个 RPC 是链 ${rpcChain}，不是 ${chain.name}（${chain.chainId}）`);
  }
  const bundler = createBundlerClient({ client, transport: http(bundlerUrl) });
  let entryPoints: readonly Address[];
  try {
    if ((await bundler.getChainId()) !== chain.chainId) throw new Error("链 ID 不符");
    entryPoints = await bundler.getSupportedEntryPoints();
  } catch {
    throw new Error("这个 Bundler 连不上，或者不是本网络的 Bundler，请换一个");
  }
  if (!entryPoints.some((e) => isAddressEqual(e, chain.entryPoint))) {
    throw new Error("这个 Bundler 不支持 EntryPoint v0.7，请换一个");
  }
  return { client, bundlerUrl };
}

/** 两次 Face ID：第一次由用户选钱包 Passkey，第二次用同一个 Passkey，据此算出公钥（./recover.ts）。 */
export async function recoverPasskey(): Promise<PasskeyPublicKey> {
  try {
    const challenge = () => crypto.getRandomValues(new Uint8Array(32));
    const first = await getAssertion(challenge());
    const second = await getAssertion(challenge(), first.credentialId);
    return recoverPasskeyKey(first, second);
  } catch (e) {
    throw humanizeError(e);
  }
}

export interface Balances {
  eth: bigint;
  usdt: bigint;
  /** 担保合约里转账失败留下的可领余额（`withdraw(token, to)` 取出） */
  credit: Record<Market, bigint>;
  deployed: boolean;
}

/** 要执行的操作；每种都只调本钱包能调的函数。 */
export type EmergencyOp =
  | { kind: "transfer"; to: Address; amount: bigint }
  | { kind: "claimTimeout" | "resolveTimeout"; market: Market; orderId: bigint }
  | { kind: "withdraw"; market: Market; to: Address };

interface Call {
  to: Address;
  abi: Abi;
  functionName: string;
  args: readonly unknown[];
}

/** 带 abi 的调用：模拟 revert 时 viem 能解码出合约错误名（errors.ts 翻译成人话）。 */
function callsFor(op: EmergencyOp): Call[] {
  switch (op.kind) {
    case "transfer":
      return [
        {
          to: token,
          abi: mockUsdtAbi,
          functionName: "transfer",
          args: [op.to, op.amount],
        } as const,
      ];
    case "withdraw":
      return [
        {
          to: MARKETS[op.market],
          abi: escrowAbi,
          functionName: "withdraw",
          args: [token, op.to],
        } as const,
      ];
    case "claimTimeout":
    case "resolveTimeout":
      return op.market === "escrow"
        ? [{ to: escrow, abi: escrowAbi, functionName: op.kind, args: [op.orderId] } as const]
        : [
            {
              to: streamEscrow,
              abi: streamEscrowAbi,
              functionName: op.kind,
              args: [op.orderId],
            } as const,
          ];
  }
}

/** 估算时给钱包临时「充」的 ETH（只在 Bundler 的模拟里，用来在钱包还没 ETH 时算出要多少）。 */
const ESTIMATE_BALANCE = parseEther("1");

export interface Quote {
  /** EntryPoint 要预扣的 ETH（上限，用剩的退回） */
  required: bigint;
}

export interface EmergencyWallet {
  address: Address;
  version: number;
  balances(): Promise<Balances>;
  quote(op: EmergencyOp): Promise<Quote>;
  /** 提交并等上链，返回交易哈希 */
  send(op: EmergencyOp): Promise<string>;
}

function open(conn: Connection, account: KernelAccount, v: WalletInitVersion): EmergencyWallet {
  const { client } = conn;
  const transport = http(conn.bundlerUrl);
  const pimlicoGas = bundlerGasPrice(transport);
  // 公共 Bundler 不一定实现 pimlico_getUserOperationGasPrice，退回节点的 EIP-1559 估价
  const estimateFeesPerGas = async (): Promise<FeesPerGas> => {
    try {
      return await pimlicoGas();
    } catch {
      const f = await client.estimateFeesPerGas();
      return { maxFeePerGas: f.maxFeePerGas, maxPriorityFeePerGas: f.maxPriorityFeePerGas };
    }
  };
  const bundler: BundlerClient = createBundlerClient({
    account,
    client,
    transport,
    userOperation: { estimateFeesPerGas },
  });
  const address = account.address;
  const credit = (market: Market) =>
    client.readContract({
      address: MARKETS[market],
      abi: escrowAbi,
      functionName: "credit",
      args: [address, token],
    });

  return {
    address,
    version: v.version,
    async balances() {
      const [eth, usdt, escrowCredit, streamCredit, code] = await Promise.all([
        client.getBalance({ address }),
        client.readContract({
          address: token,
          abi: mockUsdtAbi,
          functionName: "balanceOf",
          args: [address],
        }),
        credit("escrow"),
        credit("stream"),
        client.getCode({ address }),
      ]);
      return {
        eth,
        usdt,
        credit: { escrow: escrowCredit, stream: streamCredit },
        deployed: code !== undefined && code !== "0x",
      };
    },
    async quote(op) {
      const calls = callsFor(op);
      try {
        // 先带状态覆盖估算（钱包还没 ETH 也能算），Bundler 不支持状态覆盖时再直接估
        const op7 = await bundler
          .prepareUserOperation({
            account,
            calls,
            stateOverride: [{ address, balance: ESTIMATE_BALANCE }],
          })
          .catch(() => bundler.prepareUserOperation({ account, calls }));
        return { required: requiredPrefund(op7) };
      } catch (e) {
        throw humanizeError(e);
      }
    },
    async send(op) {
      try {
        const hash = await bundler.sendUserOperation({ account, calls: callsFor(op) });
        const receipt = await bundler.waitForUserOperationReceipt({ hash });
        if (!receipt.success) throw new Error("链上执行失败（gas 已扣），请核对订单状态后再试");
        return receipt.receipt.transactionHash;
      } catch (e) {
        throw humanizeError(e);
      }
    },
  };
}

export interface WalletCandidate {
  wallet: EmergencyWallet;
  balances: Balances;
}

/**
 * 按链配置里的每个初始化参数版本（ADR 0008，随页面发布、不问服务器）算出这个 Passkey 的钱包地址，
 * 读出余额与是否已部署。通常只有一个版本；多个版本时列出有钱或已部署的，让用户选。
 */
export async function findWallets(
  conn: Connection,
  key: PasskeyPublicKey,
): Promise<WalletCandidate[]> {
  try {
    const all = await Promise.all(
      chain.walletInitVersions.map(async (v) => {
        const wallet = open(conn, await kernelAccount(conn.client, passkeyKey(key), v), v);
        return { wallet, balances: await wallet.balances() };
      }),
    );
    const used = all.filter(
      ({ balances: b }) =>
        b.deployed || b.eth > 0n || b.usdt > 0n || b.credit.escrow > 0n || b.credit.stream > 0n,
    );
    return used.length > 0 ? used : all.slice(-1);
  } catch (e) {
    throw humanizeError(e);
  }
}
