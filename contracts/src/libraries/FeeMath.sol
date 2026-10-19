// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

/// @title FeeMath
/// @notice Fee, split and stream-unlock math shared by Escrow and StreamEscrow.
/// @dev Must match test-vectors/fees.json `rules` exactly; Rust and TypeScript implement the same vectors.
///      All amounts are USDT smallest units (6 decimals); every division floors.
library FeeMath {
    enum Payer {
        Seller,
        Buyer,
        Half
    }

    uint256 internal constant BPS = 10_000;

    error ZeroPeriods();

    /// @notice Total platform fee. A campaign rate disables the minimum fee.
    function feeTotal(uint256 amount, uint256 feeBps, uint256 minFee, bool campaign) internal pure returns (uint256) {
        uint256 fee = amount * feeBps / BPS;
        if (campaign || fee >= minFee) return fee;
        return minFee;
    }

    /// @notice Splits `total` between buyer and seller. Half: buyer gets the floor, seller the remainder.
    function split(uint256 total, Payer payer) internal pure returns (uint256 buyerPart, uint256 sellerPart) {
        if (payer == Payer.Seller) return (0, total);
        if (payer == Payer.Buyer) return (total, 0);
        buyerPart = total / 2;
        sellerPart = total - buyerPart;
    }

    /// @notice Amount the buyer transfers on createAndFund.
    function buyerPays(uint256 amount, uint256 buyerFee, uint256 buyerGas) internal pure returns (uint256) {
        return amount + buyerFee + buyerGas;
    }

    /// @notice Dispute fee prepaid by the initiator; zero-fee orders use the minimum fee as base.
    function disputeFee(uint256 fee, uint256 minFee, uint256 disputeMulBps) internal pure returns (uint256) {
        return (fee > 0 ? fee : minFee) * disputeMulBps / BPS;
    }

    /// @notice Stream principal that unlocks over periods (seller-borne fee and base gas sit in the fee pool).
    function streamNet(uint256 amount, uint256 sellerFee, uint256 sellerGas) internal pure returns (uint256) {
        return amount - sellerFee - sellerGas;
    }

    /// @notice Unlocked principal after `elapsedPeriods`; the remainder lands in the last period.
    function streamUnlocked(uint256 net, uint256 elapsedPeriods, uint256 periods) internal pure returns (uint256) {
        if (periods == 0) revert ZeroPeriods();
        if (elapsedPeriods >= periods) return net;
        return net * elapsedPeriods / periods;
    }

    /// @notice Platform fee actually charged when the seller ends up with `s` out of `base` (the principal the
    ///         fees were computed on). Fees are earned pro rata to `s`; a retained dispute fee replaces the platform
    ///         fee (at most one of the two is charged: charged = max(0, earned - retained)).
    /// @dev Scaling the floored integer buyer part (not the exact ratio) keeps both parts at or below their uncapped
    ///      values, which is what guarantees the buyer's share in a market's settlement cannot underflow.
    function feesCharged(uint256 buyerFee, uint256 sellerFee, uint256 s, uint256 base, uint256 disputeRetained)
        internal
        pure
        returns (uint256 feeB, uint256 feeS)
    {
        feeB = buyerFee * s / base;
        feeS = sellerFee * s / base;
        uint256 fee = feeB + feeS;
        if (disputeRetained > 0 && fee > 0) {
            uint256 charged = fee > disputeRetained ? fee - disputeRetained : 0;
            // slither-disable-next-line divide-before-multiply
            feeB = feeB * charged / fee;
            feeS = charged - feeB;
        }
    }

    /// @notice Net the seller receives from a claim; zero means the claim is not allowed (claimable <= claimGas).
    function claimNet(uint256 unlocked, uint256 claimed, uint256 claimGas) internal pure returns (uint256) {
        uint256 claimable = unlocked - claimed;
        if (claimable <= claimGas) return 0;
        return claimable - claimGas;
    }
}
