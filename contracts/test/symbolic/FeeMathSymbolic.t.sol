// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {FeeMath} from "../../src/libraries/FeeMath.sol";
import {SymbolicGrids} from "./SymbolicBase.sol";

/// @dev Halmos specs for FeeMath (docs/audit/freeze-checklist.md §6). Amounts are uint40 (≤ 1.09e12 units): every
///      amount that can reach these functions is ≤ MAX_ORDER_CAP = 1e11, so the narrow types still cover the whole
///      reachable domain and keep the non-linear (mul / div) queries solvable.
/// @custom:halmos --solver cvc5-int --solver-timeout-assertion 300000
contract FeeMathSymbolic is SymbolicGrids {
    uint256 internal constant MAX_AMOUNT = 100_000e6; // EscrowConfig.MAX_ORDER_CAP

    /// The seller never receives less than the signed rate implies (ceil), and the buyer overpays by < 1 unit.
    /// @custom:halmos --solver-timeout-assertion 60000
    function check_cnyFenToUsdt_ceil(uint40 priceFen, uint32 cnyPerUsdt) public pure {
        vm.assume(cnyPerUsdt >= 50_000 && cnyPerUsdt <= 100_000); // Escrow MIN/MAX_CNY_PER_USDT
        _assertCeil(priceFen, cnyPerUsdt);
    }

    /// Same property at the `_gridRate` rates, for when the fully symbolic rate does not finish.
    /// @custom:halmos --solver-timeout-assertion 60000
    function check_cnyFenToUsdt_ceil_fixedRates(uint40 priceFen, uint8 i) public pure {
        _assertCeil(priceFen, _gridRate(i));
    }

    function _assertCeil(uint256 priceFen, uint256 cnyPerUsdt) internal pure {
        uint256 usdt = FeeMath.cnyFenToUsdt(priceFen, cnyPerUsdt);
        uint256 target = priceFen * FeeMath.CNY_SCALE;
        assert(usdt * cnyPerUsdt >= target);
        assert(usdt == 0 || (usdt - 1) * cnyPerUsdt < target);
    }

    /// Fee total is exactly max(floor(amount × bps), minFee), or the raw rate during a campaign.
    function check_feeTotal(uint64 amount, uint16 feeBps, uint64 minFee, bool campaign) public pure {
        vm.assume(feeBps <= 1000);
        uint256 fee = FeeMath.feeTotal(amount, feeBps, minFee, campaign);
        uint256 raw = uint256(amount) * feeBps / BPS;
        assert(fee == (campaign || raw >= minFee ? raw : minFee));
    }

    /// Both parts of every split sum to the total; "half" gives the seller the odd unit.
    function check_split(uint256 total, uint8 p) public pure {
        vm.assume(p <= 2);
        (uint256 b, uint256 s) = FeeMath.split(total, FeeMath.Payer(p));
        assert(b + s == total);
        if (p == 2) assert(s == b || s == b + 1);
    }

    /// Unlocked principal is monotonic in elapsed periods, never exceeds `net`, and reaches `net` at the end.
    function check_streamUnlocked_monotonic(uint40 net, uint16 periods, uint16 e1, uint16 e2) public pure {
        vm.assume(periods > 0 && periods <= 8760); // StreamEscrow: hourly × 365 days at most
        vm.assume(e1 <= e2);
        uint256 a = FeeMath.streamUnlocked(net, e1, periods);
        uint256 b = FeeMath.streamUnlocked(net, e2, periods);
        assert(a <= b);
        assert(b <= net);
        assert(FeeMath.streamUnlocked(net, periods, periods) == net);
    }

    /// A claim pays exactly claimable − claimGas, or nothing when claimable ≤ claimGas.
    function check_claimNet(uint128 unlocked, uint128 claimed, uint64 claimGas) public pure {
        vm.assume(claimed <= unlocked);
        uint256 got = FeeMath.claimNet(unlocked, claimed, claimGas);
        uint256 claimable = uint256(unlocked) - claimed;
        assert(got == (claimable > claimGas ? claimable - claimGas : 0));
    }

    function _feesCharged(uint40 buyerFee, uint40 sellerFee, uint40 base, uint40 s, uint40 retained)
        internal
        pure
        returns (uint256 rawB, uint256 rawS, uint256 feeB, uint256 feeS)
    {
        vm.assume(base > 0 && base <= MAX_AMOUNT && s <= base);
        vm.assume(buyerFee <= base && sellerFee <= base);
        rawB = uint256(buyerFee) * s / base;
        rawS = uint256(sellerFee) * s / base;
        (feeB, feeS) = FeeMath.feesCharged(buyerFee, sellerFee, s, base, retained);
    }

    /// A retained dispute fee replaces the platform fee: feeB + feeS == max(0, rawB + rawS − retained).
    function check_feesCharged_total(uint40 buyerFee, uint40 sellerFee, uint40 base, uint40 s, uint40 retained)
        public
        pure
    {
        (uint256 rawB, uint256 rawS, uint256 feeB, uint256 feeS) = _feesCharged(buyerFee, sellerFee, base, s, retained);
        uint256 raw = rawB + rawS;
        assert(feeB + feeS == (raw > retained ? raw - retained : 0));
    }

    /// Without a dispute fee both parts are exactly pro rata.
    function check_feesCharged_noDispute(uint40 buyerFee, uint40 sellerFee, uint40 base, uint40 s) public pure {
        (uint256 rawB, uint256 rawS, uint256 feeB, uint256 feeS) = _feesCharged(buyerFee, sellerFee, base, s, 0);
        assert(feeB == rawB && feeS == rawS);
    }

    /// Neither part ever exceeds its pro-rata value. This is what keeps every settlement from underflowing.
    /// @custom:halmos --solver-timeout-assertion 60000
    function check_feesCharged_partsCapped(uint40 buyerFee, uint40 sellerFee, uint40 base, uint40 s, uint40 retained)
        public
        pure
    {
        (uint256 rawB, uint256 rawS, uint256 feeB, uint256 feeS) = _feesCharged(buyerFee, sellerFee, base, s, retained);
        assert(feeB <= rawB && feeS <= rawS);
    }

    /// The same, split per part and bounded to uint32 amounts (≤ 4294 USDT, above the 200 USDT launch order cap) for
    /// when the full-width check does not finish.
    function check_feesCharged_buyerPartCapped32(
        uint32 buyerFee,
        uint32 sellerFee,
        uint32 base,
        uint32 s,
        uint32 retained
    ) public pure {
        (uint256 rawB,, uint256 feeB,) = _feesCharged(buyerFee, sellerFee, base, s, retained);
        assert(feeB <= rawB);
    }

    function check_feesCharged_sellerPartCapped32(
        uint32 buyerFee,
        uint32 sellerFee,
        uint32 base,
        uint32 s,
        uint32 retained
    ) public pure {
        (, uint256 rawS,, uint256 feeS) = _feesCharged(buyerFee, sellerFee, base, s, retained);
        assert(feeS <= rawS);
    }
}
