// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {StreamEscrow} from "../src/StreamEscrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {StreamBase} from "./StreamBase.sol";

contract StreamFuzzTest is StreamBase {
    function _total() internal view returns (uint256) {
        return _bal(buyer) + _bal(seller) + _bal(feeRecipient) + _bal(address(stream)) + _bal(address(escrow));
    }

    function _pool(StreamEscrow.Order memory o) internal pure returns (uint256) {
        return uint256(o.buyerFee) + o.sellerFee + o.buyerGas + o.sellerGas;
    }

    /// Any valid amount / payer / interval / period count (5 USDT nets at least 3.9 USDT, so 39 periods always fit).
    function _newStream(uint256 amount, uint8 payer, uint8 interval, uint32 periods)
        internal
        returns (uint256 id, StreamEscrow.Order memory o)
    {
        StreamEscrow.Offer memory offer = _soffer(bound(amount, 5 * U, 200 * U), FeeMath.Payer(bound(payer, 0, 2)));
        offer.interval = StreamEscrow.Interval(bound(interval, 0, 3));
        offer.periods = uint32(bound(periods, 1, offer.interval == StreamEscrow.Interval.Month ? 12 : 39));
        id = _screate(offer);
        o = _order(id);
    }

    /// Claims at random times then finalize: the seller ends with exactly `net` minus one claim gas per claim, the
    /// fee recipient with the whole fee pool plus those claim gas fees, and nothing stays behind.
    function testFuzz_claimScheduleThenFinalize(
        uint256 amount,
        uint8 payer,
        uint8 interval,
        uint32 periods,
        uint32[6] memory gaps
    ) public {
        uint256 t0 = _total();
        (uint256 id, StreamEscrow.Order memory o) = _newStream(amount, payer, interval, periods);
        uint256 s0 = _bal(seller);
        _start(id);
        uint256 end = block.timestamp + uint256(o.interval) * o.periods;
        uint256 claims;
        for (uint256 i; i < gaps.length; i++) {
            vm.warp(block.timestamp + bound(gaps[i], 1 hours, 40 days));
            if (block.timestamp >= end) break;
            uint256 before = _order(id).claimed;
            vm.prank(seller);
            try stream.claim(id) {
                claims++;
                assertGt(_order(id).claimed, before + o.claimGas);
            } catch {}
            assertLe(_order(id).claimed, stream.unlocked(id));
        }
        vm.warp(end);
        stream.finalize(id);
        assertEq(_bal(seller) - s0, o.net - claims * o.claimGas);
        assertEq(_bal(feeRecipient), _pool(o) + claims * o.claimGas);
        assertEq(_bal(address(stream)), 0);
        assertEq(config.totalLocked(), 0);
        assertEq(_total(), t0);
    }

    /// Unlocking never grows after a dispute, whatever happens to the clock.
    function testFuzz_disputeFreezesUnlock(uint256 amount, uint8 interval, uint32 periods, uint32 at, uint32 later)
        public
    {
        (uint256 id, StreamEscrow.Order memory o) = _newStream(amount, 0, interval, periods);
        _start(id);
        uint256 duration = uint256(o.interval) * o.periods;
        vm.warp(block.timestamp + bound(at, 0, duration - 1));
        vm.prank(buyer);
        stream.dispute(id);
        uint256 frozen = stream.unlocked(id);
        assertLe(frozen, o.net);
        vm.warp(block.timestamp + bound(later, 0, 400 days));
        assertEq(stream.unlocked(id), frozen);
    }

    /// A ruling over any state conserves money, splits only what is unclaimed, and the fee recipient never gets more
    /// than the fee pool + the dispute fee + claim gas.
    function testFuzz_resolve_conserves(
        uint256 amount,
        uint8 payer,
        uint32 periods,
        uint32 claimAt,
        uint32 disputeAt,
        uint256 bps,
        bool bySeller
    ) public {
        uint256 t0 = _total();
        (uint256 id, StreamEscrow.Order memory o) = _newStream(amount, payer, 1, periods);
        _start(id);
        uint256 duration = uint256(o.interval) * o.periods;
        vm.warp(block.timestamp + bound(claimAt, 0, duration - 1));
        vm.prank(seller);
        try stream.claim(id) {} catch {}
        uint256 claimed = _order(id).claimed;
        uint256 feeBefore = _bal(feeRecipient);
        vm.warp(block.timestamp + bound(disputeAt, 0, duration - 1));
        if (block.timestamp >= _order(id).startedAt + duration) return;
        vm.prank(bySeller ? seller : buyer);
        stream.dispute(id);
        claimed = _order(id).claimed; // a seller may have paid the dispute fee from unlocked principal

        bps = bound(bps, 0, 10_000);
        uint256 s0 = _bal(seller);
        vm.prank(arbiter);
        stream.resolve(id, bps, 0);
        assertEq(_total(), t0);
        assertEq(_bal(address(stream)), 0);
        assertEq(config.totalLocked(), 0);
        assertLe(_bal(seller) - s0, o.net - claimed + o.disputeFee);
        assertLe(_bal(feeRecipient) - feeBefore, _pool(o) + o.disputeFee);
    }

    function testFuzz_acceptEnd_conserves(uint256 amount, uint8 payer, uint32 at, uint256 toBuyer, bool started)
        public
    {
        uint256 t0 = _total();
        (uint256 id, StreamEscrow.Order memory o) = _newStream(amount, payer, 1, 10);
        if (started) _start(id);
        vm.warp(block.timestamp + bound(at, 0, 2 days));
        toBuyer = bound(toBuyer, 0, o.net);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        stream.proposeEnd(id, toBuyer);
        vm.prank(seller);
        stream.acceptEnd(id, toBuyer);
        assertEq(_total(), t0);
        assertEq(_bal(address(stream)), 0);
        assertEq(_bal(seller) - s0, o.net - toBuyer);
        assertLe(_bal(feeRecipient), _pool(o));
    }

    function testFuzz_resolveTimeout_sellerGetsUnlockedAtDispute(uint256 amount, uint32 periods, uint32 at) public {
        (uint256 id, StreamEscrow.Order memory o) = _newStream(amount, 1, 1, periods); // buyer pays fees
        _start(id);
        vm.warp(block.timestamp + bound(at, 0, uint256(o.interval) * o.periods - 1));
        vm.prank(buyer);
        stream.dispute(id);
        uint256 u = stream.unlocked(id);
        vm.warp(block.timestamp + 30 days + 1);
        uint256 s0 = _bal(seller);
        stream.resolveTimeout(id);
        assertEq(_bal(seller) - s0, u + (o.net - u) / 2);
    }
}
