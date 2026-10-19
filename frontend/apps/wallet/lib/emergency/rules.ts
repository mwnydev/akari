import { LOCAL_CHAIN_ID, chain } from "../wallet/config";

// 紧急取回页（ADR 0007）的纯规则：预填的公共节点、地址检查、ETH 预付额。不依赖论坛 API 和钱包 SDK。

export interface Endpoint {
  label: string;
  url: string;
}

/** 预填的公共 RPC（只读链 + 发起模拟）。用户可以改成任何自己信任的节点。 */
const RPCS: Record<number, Endpoint[]> = {
  42161: [
    { label: "Arbitrum 官方", url: "https://arb1.arbitrum.io/rpc" },
    { label: "PublicNode", url: "https://arbitrum-one-rpc.publicnode.com" },
    { label: "dRPC", url: "https://arbitrum.drpc.org" },
  ],
  421614: [
    { label: "Arbitrum 官方", url: "https://sepolia-rollup.arbitrum.io/rpc" },
    { label: "PublicNode", url: "https://arbitrum-sepolia-rpc.publicnode.com" },
  ],
  [LOCAL_CHAIN_ID]: [{ label: "本地 anvil", url: chain.rpcUrl }],
};

/** 预填的第三方 Bundler（EntryPoint v0.7，不用 Paymaster，用户钱包自付 ETH）。 */
const BUNDLERS: Record<number, Endpoint[]> = {
  42161: [{ label: "Pimlico 公共节点", url: "https://public.pimlico.io/v2/42161/rpc" }],
  421614: [{ label: "Pimlico 公共节点", url: "https://public.pimlico.io/v2/421614/rpc" }],
  [LOCAL_CHAIN_ID]: [{ label: "本地 Alto", url: chain.bundlerUrl }],
};

export const rpcPresets = (chainId = chain.chainId): Endpoint[] => RPCS[chainId] ?? [];
export const bundlerPresets = (chainId = chain.chainId): Endpoint[] => BUNDLERS[chainId] ?? [];

/** 节点地址：必须是 https；只有本机（开发链）允许 http。返回错误提示或 null。 */
export function endpointError(input: string): string | null {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    return "请填写完整的网址，如 https://…";
  }
  const local = url.hostname === "127.0.0.1" || url.hostname === "localhost";
  if (url.protocol === "https:" || (url.protocol === "http:" && local)) return null;
  return "只能用 https 地址";
}

/** UserOperation 的 gas 上限字段（EntryPoint v0.7）。 */
export interface GasLimits {
  callGasLimit: bigint;
  verificationGasLimit: bigint;
  preVerificationGas: bigint;
  paymasterVerificationGasLimit?: bigint | undefined;
  paymasterPostOpGasLimit?: bigint | undefined;
  maxFeePerGas: bigint;
}

/**
 * 没有 Paymaster 时 EntryPoint 从钱包预扣的 ETH = 全部 gas 上限 × maxFeePerGas（EntryPoint v0.7 `_getRequiredPrefund`）。
 * 实际花费按用掉的 gas 结算，通常少得多，多扣的退回钱包。
 */
export function requiredPrefund(op: GasLimits): bigint {
  const gas =
    op.callGasLimit +
    op.verificationGasLimit +
    op.preVerificationGas +
    (op.paymasterVerificationGasLimit ?? 0n) +
    (op.paymasterPostOpGasLimit ?? 0n);
  return gas * op.maxFeePerGas;
}

/** 建议转入的 ETH：补足预付额后再留 50% 余量（gas 价格会波动），已经够了返回 0。 */
export function topUp(required: bigint, balance: bigint): bigint {
  const target = required + required / 2n;
  return balance >= required ? 0n : target - balance;
}
