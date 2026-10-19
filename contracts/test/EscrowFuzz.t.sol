// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {EscrowBase} from "./EscrowBase.sol";

contract EscrowFuzzTest is EscrowBase {
    function _total() internal view returns (uint256) {
        return _bal(buyer) + _bal(seller) + _bal(feeRecipient) + _bal(address(escrow));
    }

    function _newOrder(uint256 amount, uint8 payer, bool delivered)
        internal
        returns (uint256 id, Escrow.Order memory o)
    {
        amount = bound(amount, 5 * U, 200 * U);
        id = _create(_offer(amount, FeeMath.Payer(bound(payer, 0, 2))));
        if (delivered) _deliver(id);
        o = escrow.getOrder(id);
    }

    /// Settlement never creates or loses money, never pays the seller more than the principal,
    /// and never charges the fee recipient more than fee + base gas.
    function testFuzz_acceptEnd_conserves(uint256 amount, uint8 payer, uint256 toBuyer, bool delivered) public {
        uint256 t0 = _total();
        (uint256 id, Escrow.Order memory o) = _newOrder(amount, payer, delivered);
        toBuyer = bound(toBuyer, 0, o.amount);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        escrow.proposeEnd(id, toBuyer);
        vm.prank(seller);
        escrow.acceptEnd(id, toBuyer);
        assertEq(_total(), t0);
        assertEq(_bal(address(escrow)), 0);
        assertLe(_bal(seller) - s0, o.amount - toBuyer);
        assertLe(_bal(feeRecipient), uint256(o.buyerFee) + o.sellerFee + o.buyerGas + o.sellerGas);
    }

    function testFuzz_resolve_conserves(uint256 amount, uint8 payer, uint256 bps, bool bySeller, bool delivered)
        public
    {
        uint256 t0 = _total();
        (uint256 id, Escrow.Order memory o) = _newOrder(amount, payer, delivered);
        bps = bound(bps, 0, 10_000);
        vm.prank(bySeller ? seller : buyer);
        escrow.dispute(id);
        vm.prank(arbiter);
        escrow.resolve(id, bps, 0);
        assertEq(_total(), t0);
        assertEq(_bal(address(escrow)), 0);
        // The platform keeps at most one of {fee, retained dispute fee}, plus base gas.
        uint256 maxFee = uint256(o.buyerFee) + o.sellerFee;
        if (o.disputeFee > maxFee) maxFee = o.disputeFee;
        assertLe(_bal(feeRecipient), maxFee + o.buyerGas + o.sellerGas);
    }

    function testFuzz_release_sellerNet(uint256 amount, uint8 payer) public {
        (uint256 id, Escrow.Order memory o) = _newOrder(amount, payer, true);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        escrow.release(id);
        assertEq(_bal(seller) - s0, o.amount - o.sellerFee - o.sellerGas);
        assertEq(_bal(feeRecipient), uint256(o.buyerFee) + o.sellerFee + o.buyerGas + o.sellerGas);
    }

    function testFuzz_timeoutRefund_noFee(uint256 amount, uint8 payer) public {
        uint256 b0 = _bal(buyer);
        (uint256 id,) = _newOrder(amount, payer, false);
        vm.warp(block.timestamp + 3 days + 1);
        escrow.claimTimeout(id);
        assertEq(_bal(buyer), b0);
        assertEq(_bal(feeRecipient), 0);
    }
}
