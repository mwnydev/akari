// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {StreamEscrow} from "../../src/StreamEscrow.sol";
import {FeeMath} from "../../src/libraries/FeeMath.sol";
import {SymbolicBase} from "./SymbolicBase.sol";

/// @dev Halmos specs for StreamEscrow (docs/audit/freeze-checklist.md §6). Same shape as EscrowSymbolic: one order per
///      check, one check per ending, amounts from `_gridAmount`; each ending must succeed when its preconditions hold,
///      leave the markets holding only the seller's bond pool, and move tokens only between the buyer, the seller and
///      the fee recipient.
/// @custom:halmos --solver cvc5-int --solver-timeout-assertion 300000
contract StreamSymbolic is SymbolicBase {
    // Who opens a dispute: the buyer, the seller paying from the wallet (1), or the seller paying from the bond.
    uint8 internal constant BY_BUYER = 0;
    uint8 internal constant BY_SELLER_BOND = 2;

    struct Terms {
        uint8 size; // `_gridAmount` index
        uint8 payer;
        uint8 length; // `_gridPeriods` index
    }

    /// Period counts: one, a few, a year of months, a year of days (needs ≥ 36.5 USDT net at 0.1 USDT per period).
    function _gridPeriods(uint8 i) internal pure returns (uint16) {
        vm.assume(i < 4);
        return i == 0 ? 1 : i == 1 ? 3 : i == 2 ? 12 : 365;
    }

    /// Daily stream for `t` with a bond of half the amount. The unlock checks also vary the interval.
    function _offer(Terms memory t) internal view returns (StreamEscrow.Offer memory o) {
        o = _offer(_gridAmount(t.size), t.payer, uint8(StreamEscrow.Interval.Day), _gridPeriods(t.length));
        o.bond = o.amount / 2;
    }

    function _offer(uint256 amount, uint8 payer, uint8 interval, uint16 periods)
        internal
        view
        returns (StreamEscrow.Offer memory)
    {
        vm.assume(payer <= 2 && interval <= 3);
        return StreamEscrow.Offer({
            seller: seller,
            buyer: address(0),
            token: address(usdt),
            amount: amount,
            feePayer: FeeMath.Payer(payer),
            maxFeeBps: 1000,
            maxBaseGas: 1 * U,
            maxClaimGas: 1 * U,
            maxMinFee: 5 * U,
            bond: 0,
            lateSlashBps: 0,
            interval: StreamEscrow.Interval(interval),
            periods: periods,
            startWindow: 3 days,
            listingHash: bytes32(0),
            expiry: uint64(block.timestamp + 1 days),
            maxFills: 1,
            nonce: 1
        });
    }

    function _withInterval(Terms memory t, uint8 interval) internal view returns (StreamEscrow.Offer memory o) {
        vm.assume(interval <= 3);
        o = _offer(t);
        o.interval = StreamEscrow.Interval(interval);
    }

    /// Seller deposits the order's bond plus `extraBond` (spare bond to pay a dispute fee from), buyer opens the order.
    function _open(Terms memory t, uint256 extraBond) internal returns (uint256, StreamEscrow.Order memory) {
        return _open(_offer(t), extraBond);
    }

    function _open(StreamEscrow.Offer memory offer, uint256 extraBond)
        internal
        returns (uint256 id, StreamEscrow.Order memory o)
    {
        _depositBond(offer.bond + extraBond);
        vm.prank(BUYER);
        id = stream.createAndFund(offer, "");
        o = stream.getOrder(id);
    }

    /// Time since `start` (grid, like the amounts): nothing, just before / exactly one period, just past half way,
    /// one second before the end. All strictly before the end, where a claim would settle the order instead.
    function _elapsed(StreamEscrow.Order memory o, uint8 i) internal pure returns (uint256) {
        vm.assume(i < 5);
        uint256 p = o.interval;
        uint256 e = i == 0 ? 0 : i == 1 ? p - 1 : i == 2 ? p : i == 3 ? p * (o.periods / 2) + 1 : p * o.periods - 1;
        vm.assume(e < p * o.periods); // one period is already the end of a single-period stream
        return e;
    }

    /// Optionally start, then let `_elapsed(when)` pass and optionally claim once.
    function _run(uint256 id, bool started, uint8 when, bool claimOnce) internal {
        if (!started) {
            vm.assume(when == 0 && !claimOnce); // one path for "never started"
            return;
        }
        vm.prank(seller);
        stream.start(id);
        vm.warp(block.timestamp + _elapsed(stream.getOrder(id), when));
        if (claimOnce) {
            vm.prank(seller);
            stream.claim(id);
        }
    }

    function _dispute(uint256 id, uint8 by) internal {
        vm.assume(by <= BY_SELLER_BOND);
        vm.prank(by == BY_BUYER ? BUYER : seller);
        if (by == BY_SELLER_BOND) stream.disputeWithBond(id);
        else stream.dispute(id);
    }

    function _call(address who, bytes memory data) internal returns (bool ok) {
        vm.prank(who);
        (ok,) = address(stream).call(data);
    }

    function _pool(StreamEscrow.Order memory o) internal pure returns (uint256) {
        return uint256(o.buyerFee) + o.sellerFee + o.buyerGas + o.sellerGas;
    }

    // ───────────────────────────── Unlocking ─────────────────────────────

    /// Unlocked principal never decreases over time, never exceeds `net`, and reaches `net` when the stream ends.
    function check_unlock_monotonic(Terms memory t, uint8 interval, uint32 t1, uint32 t2) public {
        (uint256 id, StreamEscrow.Order memory o) = _open(_withInterval(t, interval), 0);
        vm.prank(seller);
        stream.start(id);
        uint256 start = block.timestamp;
        vm.assume(t1 <= t2);
        vm.warp(start + t1);
        uint256 u1 = stream.unlocked(id);
        vm.warp(start + t2);
        uint256 u2 = stream.unlocked(id);
        assert(u1 <= u2 && u2 <= o.net);
        vm.warp(start + uint256(o.interval) * o.periods);
        assert(stream.unlocked(id) == o.net);
    }

    /// After a dispute, unlocking freezes at `disputedAt` whatever time passes.
    function check_unlock_frozenByDispute(Terms memory t, uint8 interval, uint8 when, uint32 later, uint8 by) public {
        (uint256 id,) = _open(_withInterval(t, interval), 5 * U);
        _run(id, true, when, false);
        _dispute(id, by);
        uint256 frozen = stream.unlocked(id);
        vm.warp(block.timestamp + later);
        assert(stream.unlocked(id) == frozen);
    }

    /// A claim pays the seller exactly the newly unlocked part minus claim gas, never more than unlocked.
    function check_claim(Terms memory t, uint8 when) public {
        (uint256 id, StreamEscrow.Order memory o) = _open(t, 0);
        _run(id, true, when, false);
        uint256 unlockedNow = stream.unlocked(id);
        uint256 sellerBefore = usdt.balanceOf(seller);
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        vm.prank(seller);
        stream.claim(id);
        StreamEscrow.Order memory r = stream.getOrder(id);
        assert(r.claimed == unlockedNow && r.claimed <= o.net);
        assert(usdt.balanceOf(seller) - sellerBefore + (usdt.balanceOf(FEE_RECIPIENT) - feeBefore) == unlockedNow);
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore == o.claimGas);
    }

    // ───────────────────────────── Endings ─────────────────────────────

    /// Stream over: anyone finalizes; the seller ends with all of `net` (claimed + final), the platform the pool.
    function check_finalize(Terms memory t, uint8 when, bool claimOnce, uint32 late) public {
        (uint256 id, StreamEscrow.Order memory o) = _open(t, 0);
        uint256 sellerStart = usdt.balanceOf(seller);
        uint256 feeStart = usdt.balanceOf(FEE_RECIPIENT);
        _run(id, true, when, claimOnce);
        vm.warp(uint256(stream.getOrder(id).startedAt) + uint256(o.interval) * o.periods + late);
        uint256 before = _allowedHoldings();
        assert(_call(address(0xCAFE), abi.encodeCall(stream.finalize, (id))));
        uint256 claimGasPaid = claimOnce ? o.claimGas : 0;
        assert(usdt.balanceOf(seller) - sellerStart == uint256(o.net) - claimGasPaid);
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeStart == _pool(o) + claimGasPaid);
        _assertAllSettled(before);
    }

    /// Seller refunds what is unclaimed; fees only on what was already claimed, the bond is not slashed.
    function check_refund(Terms memory t, bool started, uint8 when, bool claimOnce) public {
        (uint256 id, StreamEscrow.Order memory o) = _open(t, 0);
        _run(id, started, when, claimOnce);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        uint256 bondBefore = _bondTotal();
        assert(_call(seller, abi.encodeCall(stream.refund, (id))));
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore <= _pool(o));
        assert(_bondTotal() == bondBefore);
        _assertAllSettled(before);
    }

    /// Not started by the deadline: anyone refunds the buyer in full plus the agreed share of the bond.
    function check_claimTimeout(Terms memory t, uint8 bondShare, uint8 slashShare, uint32 late) public {
        StreamEscrow.Offer memory offer = _offer(t);
        offer.bond = _share(offer.amount, bondShare);
        offer.lateSlashBps = _gridBps(slashShare);
        (uint256 id, StreamEscrow.Order memory o) = _open(offer, 0);
        vm.warp(uint256(o.startDeadline) + 1 + late);
        uint256 before = _allowedHoldings();
        uint256 buyerBefore = usdt.balanceOf(BUYER);
        assert(_call(address(0xCAFE), abi.encodeCall(stream.claimTimeout, (id))));
        uint256 paid = uint256(o.net) + _pool(o);
        assert(usdt.balanceOf(BUYER) - buyerBefore == paid + uint256(o.bond) * o.lateSlashBps / BPS);
        _assertAllSettled(before);
    }

    /// Mutual settlement over the unclaimed part, optionally during a dispute: always executable.
    function check_acceptEnd(
        Terms memory t,
        bool started,
        uint8 when,
        bool disputed,
        uint8 by,
        uint8 buyerShare,
        bool sellerProposes
    ) public {
        (uint256 id, StreamEscrow.Order memory o) = _open(t, 5 * U);
        _run(id, started, when, false);
        if (disputed) _dispute(id, by);
        uint256 toBuyer = _share(o.net - stream.getOrder(id).claimed, buyerShare);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        address proposer = sellerProposes ? seller : BUYER;
        address acceptor = sellerProposes ? BUYER : seller;
        assert(_call(proposer, abi.encodeCall(stream.proposeEnd, (id, toBuyer))));
        assert(_call(acceptor, abi.encodeCall(stream.acceptEnd, (id, toBuyer))));
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore <= _pool(o));
        _assertAllSettled(before);
    }

    /// Arbiter ruling over everything unclaimed: always executable; the platform keeps gas plus at most one of
    /// {platform fee, dispute fee}.
    function check_resolve(Terms memory t, bool started, uint8 when, uint8 by, uint8 split, uint8 bondSplit) public {
        uint256 toBuyerBps = _gridBps(split);
        vm.assume(bondSplit % 2 == 0); // bond ruling: none, half or all
        uint256 bondToBuyerBps = _gridBps(bondSplit);
        (uint256 id, StreamEscrow.Order memory o) = _open(t, 5 * U);
        _run(id, started, when, false);
        _dispute(id, by);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        assert(_call(ARBITER, abi.encodeCall(stream.resolve, (id, toBuyerBps, bondToBuyerBps))));
        uint256 fees = uint256(o.buyerFee) + o.sellerFee;
        uint256 maxFee = fees > o.disputeFee ? fees : o.disputeFee;
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore <= uint256(o.buyerGas) + o.sellerGas + maxFee);
        _assertAllSettled(before);
    }

    /// Unresolved dispute after the timeout: anyone settles; dispute fee refunded, platform fee only.
    function check_resolveTimeout(Terms memory t, bool started, uint8 when, uint8 by, uint32 late) public {
        (uint256 id, StreamEscrow.Order memory o) = _open(t, 5 * U);
        _run(id, started, when, false);
        _dispute(id, by);
        StreamEscrow.Order memory d = stream.getOrder(id);
        vm.warp(uint256(d.disputedAt) + d.resolveTimeout + 1 + late);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        assert(_call(address(0xCAFE), abi.encodeCall(stream.resolveTimeout, (id))));
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore <= _pool(o));
        _assertAllSettled(before);
    }

    // ───────────────────────────── Caps ─────────────────────────────

    /// Stream orders count toward the same per-order and total caps.
    function check_caps(uint40 bond, uint40 amount, uint8 payer, uint8 interval, uint16 periods) public {
        vm.prank(seller);
        (bool ok,) = address(escrow).call(abi.encodeCall(escrow.depositBond, (address(usdt), bond)));
        StreamEscrow.Offer memory o = _offer(amount, payer, interval, periods);
        vm.prank(BUYER);
        (ok,) = address(stream).call(abi.encodeCall(stream.createAndFund, (o, "")));
        StreamEscrow.Order memory r = stream.getOrder(1);
        if (ok) assert(uint256(r.net) + r.sellerFee + r.sellerGas <= ORDER_CAP);
        assert(config.totalLocked() <= TOTAL_CAP);
    }
}
