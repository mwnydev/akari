// ⚠️ 仅限本地开发链（deploy/dev/aa.sh）。只经 sponsor.ts 里 `if (DEV_SPONSOR)` 分支动态加载；
// `next build` 遇到 NEXT_PUBLIC_DEV_SPONSOR=1 会直接失败（next.config.ts），所以不会出现在任何部署产物里。
// 生产的代付签名只在 forum-sponsor（后端独立进程）里完成，前端永远不持有 Paymaster 签名私钥。
import { fixedFeePaymasterAbi } from "@akari/contracts";
import { createPublicClient, encodePacked, http, type Hex } from "viem";
import { toPackedUserOperation, type GetPaymasterDataParameters } from "viem/account-abstraction";
import { privateKeyToAccount } from "viem/accounts";
import { chain } from "./config";
import { sponsorFeeFor, type PaymasterFields, type SponsorClient } from "./sponsor";

/** cast keccak "akari-dev-only:sponsor"——公开的开发密钥，aa.sh 把它的地址设为本地 Paymaster 的 signer。 */
const DEV_SPONSOR_KEY: Hex = "0xdccc2a137d5ed46c4b46ea12b6e50feb5565cfa60bda1c3814b0e25d3f52eb98";

/** 验证阶段要做 ECDSA 恢复 + USDT transferFrom；估算时占位签名无效、走不到转账，所以固定给足而不用估算值。 */
const VERIFICATION_GAS = 120_000n;
/** forum-sponsor 签 2 分钟（plan 第 5 节）；validAfter 往前留 30 秒容忍时钟偏差，窗口远小于合约上限 10 分钟。 */
const VALIDITY_SECONDS = 120;
const CLOCK_SKEW_SECONDS = 30;

function encodeData(validUntil: number, validAfter: number, fee: bigint, signature: Hex): Hex {
  return encodePacked(
    ["uint48", "uint48", "uint128", "bytes"],
    [validUntil, validAfter, fee, signature],
  );
}

/** viem 在 getPaymasterData 时已经填好全部 gas 字段；缺字段说明调用顺序不对，宁可失败也不签不完整的操作。 */
function completeUserOperation(op: GetPaymasterDataParameters) {
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

export function createDevSponsorClient(): SponsorClient {
  const signer = privateKeyToAccount(DEV_SPONSOR_KEY);
  if (signer.address !== chain.paymasterSigner) {
    throw new Error(
      "DEV 代付密钥与本地 Paymaster 的 signer 不一致：请重新运行 deploy/dev/aa.sh up",
    );
  }
  const publicClient = createPublicClient({ transport: http(chain.rpcUrl) });
  const paymaster = chain.contracts.FixedFeePaymaster;

  const fields = (paymasterData: Hex): PaymasterFields => ({
    paymaster,
    paymasterData,
    paymasterVerificationGasLimit: VERIFICATION_GAS,
    paymasterPostOpGasLimit: 0n, // 合约不实现 postOp
  });

  return {
    async getPaymasterStubData(action) {
      const now = Math.floor(Date.now() / 1000);
      return fields(
        encodeData(now + VALIDITY_SECONDS, now, sponsorFeeFor(action), `0x${"00".repeat(65)}`),
      );
    },
    async getPaymasterData(action, params) {
      const validAfter = Math.floor(Date.now() / 1000) - CLOCK_SKEW_SECONDS;
      const validUntil = validAfter + CLOCK_SKEW_SECONDS + VALIDITY_SECONDS;
      const fee = sponsorFeeFor(action);
      const packed = toPackedUserOperation({
        ...completeUserOperation(params),
        ...fields("0x"),
        signature: "0x",
      });
      // 哈希直接问合约（getHash 是唯一定义），不在前端重写一遍
      const hash = await publicClient.readContract({
        address: paymaster,
        abi: fixedFeePaymasterAbi,
        functionName: "getHash",
        args: [packed, validUntil, validAfter, fee],
      });
      const signature = await signer.signMessage({ message: { raw: hash } });
      return fields(encodeData(validUntil, validAfter, fee, signature));
    },
  };
}
