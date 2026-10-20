// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {MockMarket, IBondPool} from "./mocks/TestTokens.sol";
import {EscrowBase} from "./EscrowBase.sol";

/// Seller bond pool (plan §6「卖家保证金」), the global locked cap and arbiter deadline extensions.
contract EscrowBondTest is EscrowBase {
    function _bondOffer(uint256 bond, uint16 lateSlashBps) internal view returns (Escrow.Offer memory o) {
        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.bond = bond;
        o.lateSlashBps = lateSlashBps;
    }

    function test_depositWithdraw() public {
        _depositBond(50 * U);
        (uint256 t, uint256 l, uint256 a) = _bond();
        assertEq(t, 50 * U);
        assertEq(l, 0);
        assertEq(a, 50 * U);
        assertEq(config.totalLocked(), 50 * U);

        address fresh = makeAddr("fresh");
        vm.prank(seller);
        escrow.withdrawBond(address(usdt), 20 * U, fresh);
        assertEq(_bal(fresh), 20 * U);
        (t,,) = _bond();
        assertEq(t, 30 * U);
        assertEq(config.totalLocked(), 30 * U);
        _depositBond(25 * U); // deposits add up
        (t,,) = _bond();
        assertEq(t, 55 * U);
    }

    function test_revert_depositWithdraw() public {
        vm.startPrank(seller);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.depositBond(address(usdt), 0);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.depositBond(makeAddr("notAToken"), 1);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.depositBond(makeAddr("notAToken"), 0);
        escrow.depositBond(address(usdt), 10 * U);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        escrow.withdrawBond(address(usdt), 0, seller);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        escrow.withdrawBond(address(usdt), 10 * U + 1, seller);
        vm.stopPrank();
    }

    function test_createLocksBond_andRejectsWhenInsufficient() public {
        _depositBond(100 * U);
        uint256 id = _create(_bondOffer(60 * U, 0));
        assertEq(escrow.getOrder(id).bond, 60 * U);
        (uint256 t, uint256 l, uint256 a) = _bond();
        assertEq(t, 100 * U);
        assertEq(l, 60 * U);
        assertEq(a, 40 * U);

        Escrow.Offer memory o = _bondOffer(50 * U, 0);
        o.nonce = 2;
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        escrow.createAndFund(o, sig);

        // locked part cannot be withdrawn
        vm.prank(seller);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        escrow.withdrawBond(address(usdt), 41 * U, seller);

        // exactly the free part can back a second order; locks add up and release one by one
        o.bond = 40 * U;
        o.nonce = 3;
        uint256 id2 = _create(o);
        (, l, a) = _bond();
        assertEq(l, 100 * U);
        assertEq(a, 0);
        vm.prank(buyer);
        escrow.release(id);
        (, l,) = _bond();
        assertEq(l, 40 * U);
        vm.prank(buyer);
        escrow.release(id2);
        (, l,) = _bond();
        assertEq(l, 0);
    }

    function test_revert_create_badBondTerms() public {
        _depositBond(500 * U);
        Escrow.Offer memory o = _bondOffer(100 * U + 1, 0); // bond > amount
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadOffer.selector);
        escrow.createAndFund(o, sig);

        o = _bondOffer(10 * U, 10_001);
        sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadOffer.selector);
        escrow.createAndFund(o, sig);
    }

    function test_release_unlocksBond() public {
        _depositBond(100 * U);
        uint256 id = _create(_bondOffer(60 * U, 5000));
        vm.prank(buyer);
        escrow.release(id);
        (uint256 t, uint256 l,) = _bond();
        assertEq(t, 100 * U);
        assertEq(l, 0);
    }

    function test_refund_unlocksBond_noSlash() public {
        _depositBond(100 * U);
        uint256 id = _create(_bondOffer(60 * U, 5000));
        vm.prank(seller);
        escrow.refund(id);
        (uint256 t, uint256 l,) = _bond();
        assertEq(t, 100 * U);
        assertEq(l, 0);
    }

    function test_lateShipping_slashesToBuyer() public {
        _depositBond(100 * U);
        uint256 b0 = _bal(buyer);
        uint256 id = _create(_bondOffer(40 * U, 2500));
        vm.warp(block.timestamp + 3 days + 1);
        escrow.claimTimeout(id);
        assertEq(_bal(buyer) - b0, 10 * U); // principal back + 25% of the 40 bond
        (uint256 t, uint256 l,) = _bond();
        assertEq(t, 90 * U);
        assertEq(l, 0);
        assertEq(config.totalLocked(), 90 * U);
    }

    function test_resolve_bondToBuyer() public {
        _depositBond(100 * U);
        uint256 id = _create(_bondOffer(80 * U, 0));
        _deliver(id);
        vm.prank(buyer);
        escrow.dispute(id);
        uint256 b0 = _bal(buyer);
        vm.prank(arbiter);
        escrow.resolve(id, 10_000, 5000);
        // principal + dispute fee back, minus base gas (delivered), plus half the bond
        assertEq(_bal(buyer) - b0, 100 * U + 1 * U - 100_000 + 40 * U);
        (uint256 t, uint256 l,) = _bond();
        assertEq(t, 60 * U);
        assertEq(l, 0);
    }

    function test_revert_resolve_bondBps() public {
        uint256 id = _createDefault();
        vm.prank(buyer);
        escrow.dispute(id);
        vm.prank(arbiter);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.resolve(id, 0, 10_001);
    }

    function test_disputeWithBond_sellerWins_feeBackToBond() public {
        _depositBond(10 * U);
        uint256 id = _createDefault();
        _deliver(id);
        uint256 s0 = _bal(seller);
        vm.prank(seller);
        escrow.disputeWithBond(id);
        assertEq(_bal(seller), s0); // nothing pulled from the wallet
        (uint256 t,,) = _bond();
        assertEq(t, 9 * U);
        assertTrue(escrow.getOrder(id).disputeFromBond);
        vm.prank(arbiter);
        escrow.resolve(id, 0, 0);
        (t,,) = _bond();
        assertEq(t, 10 * U); // refunded into the bond, not the wallet
        assertEq(config.totalLocked(), 10 * U);
    }

    function test_disputeWithBond_sellerLoses_feeRetained() public {
        _depositBond(10 * U);
        uint256 id = _createDefault();
        _deliver(id);
        vm.prank(seller);
        escrow.disputeWithBond(id);
        vm.prank(arbiter);
        escrow.resolve(id, 10_000, 0);
        (uint256 t,,) = _bond();
        assertEq(t, 9 * U);
        assertEq(_bal(feeRecipient), 1 * U + 100_000); // retained dispute fee + base gas (no platform fee)
    }

    function test_revert_disputeWithBond() public {
        uint256 id = _createDefault();
        vm.prank(buyer);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.disputeWithBond(id);
        vm.prank(seller);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        escrow.disputeWithBond(id);
        _depositBond(1 * U); // exactly the dispute fee is enough
        vm.prank(seller);
        escrow.disputeWithBond(id);
    }

    function test_acceptEnd_and_resolveTimeout_unlockBond() public {
        _depositBond(100 * U);
        uint256 id = _create(_bondOffer(50 * U, 10_000));
        vm.prank(buyer);
        escrow.proposeEnd(id, 30 * U);
        vm.prank(seller);
        escrow.acceptEnd(id, 30 * U);
        (uint256 t, uint256 l,) = _bond();
        assertEq(t, 100 * U);
        assertEq(l, 0);

        Escrow.Offer memory o = _bondOffer(50 * U, 10_000);
        o.nonce = 2;
        id = _create(o);
        vm.prank(buyer);
        escrow.dispute(id);
        vm.warp(block.timestamp + 30 days + 1);
        escrow.resolveTimeout(id);
        (t, l,) = _bond();
        assertEq(t, 100 * U);
        assertEq(l, 0);
    }

    // ───────────── second market (StreamEscrow) ─────────────

    function _addMarket() internal returns (MockMarket m) {
        m = new MockMarket();
        vm.prank(owner);
        config.queueMarket(address(m));
        vm.warp(block.timestamp + 72 hours);
        config.addMarket(address(m));
        assertTrue(config.isMarket(address(m)));
    }

    function test_market_lockAndRelease() public {
        _depositBond(100 * U);
        MockMarket m = _addMarket();
        IBondPool pool = IBondPool(address(escrow));
        m.lock(pool, seller, address(usdt), 30 * U);
        m.lock(pool, seller, address(usdt), 20 * U);
        assertEq(escrow.lockedBy(address(m), seller, address(usdt)), 50 * U);
        (, uint256 l,) = _bond();
        assertEq(l, 50 * U);

        uint256 b0 = _bal(buyer);
        m.release(pool, seller, address(usdt), 40 * U, buyer, 5 * U);
        assertEq(_bal(buyer) - b0, 5 * U);
        assertEq(escrow.lockedBy(address(m), seller, address(usdt)), 10 * U);
        (, l,) = _bond();
        assertEq(l, 10 * U);
        m.release(pool, seller, address(usdt), 10 * U, buyer, 10 * U); // all of it to the buyer
        assertEq(_bal(buyer) - b0, 15 * U);
        (uint256 t, uint256 l2,) = _bond();
        assertEq(t, 85 * U);
        assertEq(l2, 0);
        assertEq(config.totalLocked(), 85 * U);
    }

    function test_revert_market() public {
        _depositBond(10 * U);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.lockBond(seller, address(usdt), 1);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.releaseBond(seller, address(usdt), 1, buyer, 0);

        MockMarket m = _addMarket();
        IBondPool pool = IBondPool(address(escrow));
        vm.expectRevert(Escrow.InsufficientBond.selector);
        m.lock(pool, seller, address(usdt), 10 * U + 1);
        m.lock(pool, seller, address(usdt), 5 * U);
        vm.expectRevert(Escrow.BadParams.selector);
        m.release(pool, seller, address(usdt), 1 * U, buyer, 1 * U + 1);
        vm.expectRevert(); // arithmetic underflow: a market can only release what it locked
        m.release(pool, seller, address(usdt), 5 * U + 1, buyer, 0);
    }

    /// A market can take only the seller's free bond (its dispute fee); custody stays counted, tokens move.
    function test_market_takeBond() public {
        _depositBond(10 * U);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.takeBond(seller, address(usdt), 1);

        MockMarket m = _addMarket();
        IBondPool pool = IBondPool(address(escrow));
        m.lock(pool, seller, address(usdt), 6 * U);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        m.take(pool, seller, address(usdt), 4 * U + 1); // locked part is off limits
        m.take(pool, seller, address(usdt), 4 * U);
        assertEq(_bal(address(m)), 4 * U);
        (uint256 t, uint256 l, uint256 a) = _bond();
        assertEq(t, 6 * U);
        assertEq(l, 6 * U);
        assertEq(a, 0);
        assertEq(config.totalLocked(), 10 * U);
    }

    // ───────────── global locked cap ─────────────

    function _setTotalCap(uint256 cap) internal {
        EscrowConfig.Params memory p = defaultParams();
        p.totalLockedCap = uint64(cap);
        vm.prank(owner);
        config.setParams(p);
    }

    function test_totalLockedCap_blocksNewOrdersAndBonds_notDisputes() public {
        _setTotalCap(250 * U);
        uint256 id = _createDefault();
        Escrow.Offer memory o = _offer(150 * U, FeeMath.Payer.Seller);
        o.nonce = 2;
        _create(o);
        assertEq(config.totalLocked(), 250 * U);

        o = _offer(5 * U, FeeMath.Payer.Seller);
        o.nonce = 3;
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(EscrowConfig.TotalLockedCap.selector);
        escrow.createAndFund(o, sig);

        vm.prank(seller);
        vm.expectRevert(EscrowConfig.TotalLockedCap.selector);
        escrow.depositBond(address(usdt), 1);

        vm.prank(buyer);
        escrow.dispute(id); // never blocked by the cap
        assertEq(config.totalLocked(), 251 * U);

        vm.prank(arbiter);
        escrow.resolve(id, 10_000, 0);
        assertEq(config.totalLocked(), 150 * U);
    }

    function test_revert_lockFunds_notMarket() public {
        vm.expectRevert(EscrowConfig.NotMarket.selector);
        config.lockFunds(1, true);
        vm.expectRevert(EscrowConfig.NotMarket.selector);
        config.unlockFunds(1);
        vm.expectRevert(EscrowConfig.NotMarket.selector);
        config.checkout(address(usdt), 1);
    }

    // ───────────── arbiter deadline extension ─────────────

    function test_extendDeadlines() public {
        uint256 a = _createDefault();
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.nonce = 2;
        uint256 b = _create(o);
        _deliver(b);
        o.nonce = 3;
        uint256 c = _create(o);
        vm.prank(buyer);
        escrow.dispute(c);

        uint64 ship0 = escrow.getOrder(a).shipDeadline;
        uint64 conf0 = escrow.getOrder(b).confirmDeadline;
        uint256[] memory ids = new uint256[](4);
        (ids[0], ids[1], ids[2], ids[3]) = (a, b, c, 999);
        vm.startPrank(arbiter);
        escrow.extendDeadlines(ids, 7 days);
        escrow.extendDeadlines(ids, 7 days);
        escrow.extendDeadlines(ids, 1); // over the 14-day cap: skipped
        vm.stopPrank();
        assertEq(escrow.getOrder(a).shipDeadline, ship0 + 14 days);
        assertEq(escrow.getOrder(b).confirmDeadline, conf0 + 14 days);
        assertEq(escrow.getOrder(a).arbExtended, 14 days);
        assertEq(escrow.getOrder(c).arbExtended, 0); // disputed: skipped

        vm.prank(seller);
        escrow.refund(a);
        vm.prank(arbiter);
        escrow.extendDeadlines(ids, 1); // settled and over the cap: still skipped
        assertEq(escrow.getOrder(a).shipDeadline, ship0 + 14 days);
    }

    function test_revert_extendDeadlines() public {
        uint256[] memory ids = new uint256[](0);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.extendDeadlines(ids, 1 days);
        vm.startPrank(arbiter);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.extendDeadlines(ids, 0);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.extendDeadlines(ids, 7 days + 1);
        vm.stopPrank();
    }
}
