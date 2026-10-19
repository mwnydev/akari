import { mockUsdtAbi, type WalletInitVersion } from "@akari/contracts";
import { PasskeyValidatorContractVersion, toPasskeyValidator } from "@zerodev/passkey-validator";
import { createKernelAccount, encodeCallDataEpV07 } from "@zerodev/sdk";
import { KERNEL_V3_3 } from "@zerodev/sdk/constants";
import type { WebAuthnKey } from "@zerodev/webauthn-key";
import {
  createClient,
  encodeFunctionData,
  hexToBigInt,
  rpcSchema,
  type Hex,
  type PublicClient,
  type Transport,
} from "viem";

// Kernel v3.3 智能账户的组装（钱包 SDK 与紧急取回页共用，不依赖论坛 API）。体积大，只能在动态加载的分包里用。

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
export async function kernelAccount(client: PublicClient, key: WebAuthnKey, v: WalletInitVersion) {
  const entryPoint = { address: v.entryPoint, version: "0.7" } as const;
  const validator = await toPasskeyValidator(client, {
    webAuthnKey: key,
    entryPoint,
    kernelVersion: KERNEL_V3_3,
    validatorContractVersion: PasskeyValidatorContractVersion.V0_0_3_PATCHED,
    validatorAddress: v.webauthnValidator,
  });
  return createKernelAccount(client, {
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

export type KernelAccount = Awaited<ReturnType<typeof kernelAccount>>;

export interface FeesPerGas {
  maxFeePerGas: bigint;
  maxPriorityFeePerGas: bigint;
}

type GasPriceTier = { maxFeePerGas: Hex; maxPriorityFeePerGas: Hex };

/** bundler 的 `pimlico_getUserOperationGasPrice`（forum-sponsor 与 Alto、Pimlico 都支持）取「standard」档。 */
export function bundlerGasPrice(transport: Transport): () => Promise<FeesPerGas> {
  const rpc = createClient({
    transport,
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
  return async () => {
    const { standard } = await rpc.request({
      method: "pimlico_getUserOperationGasPrice",
      params: [],
    });
    return {
      maxFeePerGas: hexToBigInt(standard.maxFeePerGas),
      maxPriorityFeePerGas: hexToBigInt(standard.maxPriorityFeePerGas),
    };
  };
}
