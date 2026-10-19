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

/** EIP-712 `AuctionAuth` 与 `SettlementProof`，同样由 `createFromAuction` 的参数推出（`typed-data.test.ts` 核对 TYPEHASH）。 */
const createFromAuction = getAbiItem({ abi: escrowAbi, name: "createFromAuction" });

export const AUCTION_TYPES = {
  AuctionAuth: createFromAuction.inputs[0].components.map(({ name, type }) => ({ name, type })),
};

export const PROOF_TYPES = {
  SettlementProof: createFromAuction.inputs[2].components.map(({ name, type }) => ({ name, type })),
};
