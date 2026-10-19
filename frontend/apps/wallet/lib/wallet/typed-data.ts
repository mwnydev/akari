import { escrowAbi } from "@akari/contracts";
import { getAbiItem } from "viem";

/**
 * EIP-712 `Offer` 类型由生成的 ABI（`createAndFund` 的 offer 参数）推出，字段顺序与 Solidity 结构体一致，不手抄。
 * `typed-data.test.ts` 核对它与 Escrow.sol 里的 OFFER_TYPEHASH 字符串完全相同。
 */
const createAndFund = getAbiItem({ abi: escrowAbi, name: "createAndFund" });

export const OFFER_TYPES = {
  Offer: createAndFund.inputs[0].components.map(({ name, type }) => ({ name, type })),
};
