// 合约 ABI 由 @wagmi/cli 从 contracts/ 的 Foundry 产物生成（`pnpm --filter @akari/contracts generate`），勿手写。
export { escrowAbi, escrowConfigAbi, fixedFeePaymasterAbi, mockUsdtAbi } from "./generated";
export { type ChainConfig, localChain } from "./chain";
