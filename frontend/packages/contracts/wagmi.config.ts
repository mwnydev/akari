import { defineConfig } from "@wagmi/cli";
import { foundry } from "@wagmi/cli/plugins";

// 合约 ABI 的唯一来源是 contracts/ 的 Solidity 源码（plan 第 14 节「单一事实来源」）。
// `pnpm generate` 先跑 `forge build` 再重新生成 src/generated.ts；生成文件入库，CI 不需要 Foundry。
export default defineConfig({
  out: "src/generated.ts",
  plugins: [
    foundry({
      project: "../../../contracts",
      include: [
        "Escrow.sol/**",
        "StreamEscrow.sol/**",
        "EscrowConfig.sol/**",
        "MockUSDT.sol/**",
        "FixedFeePaymaster.sol/**",
      ],
      forge: { build: true, rebuild: true },
    }),
  ],
});
