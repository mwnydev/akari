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
  /** 钱包初始化参数的全部版本（ADR 0008），按版本号升序，只增不改。 */
  walletInitVersions: WalletInitVersion[];
}

/**
 * 一个钱包初始化参数版本（ADR 0008）：决定 Kernel 账户的 CREATE2 地址。字段名与
 * `GET /api/v1/trade/params` 的 `wallet_init_versions` 相同；`approveAmount` 为十进制字符串。
 */
export interface WalletInitVersion {
  version: number;
  kernelImpl: Address;
  factory: Address;
  metaFactory: Address;
  webauthnValidator: Address;
  entryPoint: Address;
  paymaster: Address;
  approveToken: Address;
  approveAmount: string;
}

/** uint256 最大值：initCode 里给 Paymaster 的 USDT 授权额度。 */
const MAX_UINT256 = (2n ** 256n - 1n).toString();

/** 本地链每次 `aa.sh up` 重新部署，只有版本 1，参数直接取自生成的 chain.local.json。 */
export const localChain: ChainConfig = {
  ...(local as Omit<ChainConfig, "walletInitVersions">),
  walletInitVersions: [
    {
      version: 1,
      kernelImpl: local.kernel.implementation as Address,
      factory: local.kernel.factory as Address,
      metaFactory: local.kernel.metaFactory as Address,
      webauthnValidator: local.webAuthnValidator as Address,
      entryPoint: local.entryPoint as Address,
      paymaster: local.contracts.FixedFeePaymaster as Address,
      approveToken: local.contracts.MockUSDT as Address,
      approveAmount: MAX_UINT256,
    },
  ],
};
