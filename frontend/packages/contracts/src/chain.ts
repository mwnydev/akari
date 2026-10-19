import type { Address } from "viem";
import local from "./chain.local.json" with { type: "json" };

/** 一条链上钱包需要的全部地址。本地链由 deploy/dev/aa.sh 生成；测试网 / 主网配置接入时按同一形状加。 */
export interface ChainConfig {
  chainId: number;
  name: string;
  rpcUrl: string;
  bundlerUrl: string;
  entryPoint: Address;
  kernel: { version: "0.3.3"; implementation: Address; factory: Address; metaFactory: Address };
  webAuthnValidator: Address;
  erc6492Verifier: Address;
  contracts: {
    MockUSDT: Address;
    EscrowConfig: Address;
    Escrow: Address;
    FixedFeePaymaster: Address;
  };
  /** Paymaster 合约里登记的代付签名者（只有地址；私钥在 forum-sponsor，本地链见 DEV 代付实现）。 */
  paymasterSigner: Address;
}

export const localChain = local as ChainConfig;
