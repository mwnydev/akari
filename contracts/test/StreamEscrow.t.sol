// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {MarketBase} from "../src/MarketBase.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {Vm} from "forge-std/Vm.sol";
import {FeeOnTransferToken} from "./mocks/TestTokens.sol";
import {MockAccountFactory} from "./mocks/MockAccount.sol";
import {StreamBase} from "./StreamBase.sol";

contract StreamCreateTest is StreamBase {
    function _expectCreateRevert(StreamEscrow.Offer memory o, bytes4 sel) internal {
        bytes memory sig = _ssign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(sel);
        stream.createAndFund(o, sig);
    }

    function test_constructor_readsEscrow() public {
        assertEq(address(stream.escrow()), address(escrow));
        assertEq(address(stream.config()), address(config));
        assertEq(stream.arbiter(), arbiter);
        assertEq(stream.feeRecipient(), feeRecipient);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        new StreamEscrow(Escrow(address(0)));
    }

    function test_createAndFund_sellerPays() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        bytes memory sig = _ssign(o, sellerKey);
        uint256 b0 = _bal(buyer);
        vm.expectEmit(address(stream));
        emit StreamEscrow.OrderCreated(1, stream.hashOffer(o), buyer, seller, 100 * U, o.listingHash, 1 days, 10);
        vm.prank(buyer);
        uint256 id = stream.createAndFund(o, sig);
        assertEq(id, 1);
        assertEq(b0 - _bal(buyer), 100 * U);
        assertEq(_bal(address(stream)), 100 * U);
        assertEq(config.totalLocked(), 100 * U);
        assertEq(stream.fills(stream.hashOffer(o)), 1);

        StreamEscrow.Order memory s = _order(id);
        assertEq(uint8(s.state), uint8(StreamEscrow.State.Funded));
        assertEq(s.buyer, buyer);
        assertEq(s.seller, seller);
        assertEq(s.net, NET);
        assertEq(s.sellerFee, 1 * U);
        assertEq(s.sellerGas, 100_000);
        assertEq(s.buyerFee, 0);
        assertEq(s.buyerGas, 0);
        assertEq(s.claimGas, CLAIM_GAS);
        assertEq(s.disputeFee, DISPUTE_FEE);
        assertEq(s.interval, 1 days);
        assertEq(s.periods, 10);
        assertEq(s.startDeadline, block.timestamp + 3 days);
        assertEq(s.startedAt, 0);
        assertEq(s.resolveTimeout, 30 days);
        assertEq(s.proposalWindow, 72 hours);
        assertFalse(s.platformPaid);
        assertEq(stream.unlocked(id), 0);
    }

    function test_createAndFund_buyerAndHalfPay() public {
        uint256 b0 = _bal(buyer);
        uint256 id = _screate(_soffer(100 * U, FeeMath.Payer.Buyer));
        assertEq(b0 - _bal(buyer), 101_100_000);
        assertEq(_order(id).net, 100 * U);

        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Half);
        o.nonce = 2;
        id = _screate(o);
        assertEq(_order(id).net, 99_450_000);
        assertEq(_order(id).buyerFee, 500_000);
        assertEq(_order(id).buyerGas, 50_000);
    }

    function test_createAndFund_privateOffer() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.buyer = stranger;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.buyer = address(1); // designated buyers below and above the caller alike
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.buyer = buyer;
        _screate(o);
    }

    /// The self-dealing check compares addresses for equality only: any buyer other than the seller may fill.
    function test_createAndFund_buyerAboveSeller() public {
        address high = address(type(uint160).max);
        _fund(high, 1000 * U);
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        bytes memory sig = _ssign(o, sellerKey);
        vm.prank(high);
        stream.createAndFund(o, sig);
    }

    function test_revert_offerUnavailable() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        _screate(o);
        _expectCreateRevert(o, MarketBase.OfferUnavailable.selector); // maxFills used up

        o.nonce = 2;
        vm.prank(seller);
        stream.cancelOffer(2);
        assertTrue(stream.cancelledNonce(seller, 2));
        _expectCreateRevert(o, MarketBase.OfferUnavailable.selector);

        o.nonce = 3;
        vm.warp(o.expiry + 1);
        _expectCreateRevert(o, MarketBase.OfferUnavailable.selector);
    }

    function test_revert_badSignature() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        (, uint256 otherKey) = makeAddrAndKey("other");
        bytes memory sig = _ssign(o, otherKey);
        vm.prank(buyer);
        vm.expectRevert(MarketBase.BadSignature.selector);
        stream.createAndFund(o, sig);
    }

    function test_createAndFund_undeployedAccountSeller_erc6492() public {
        bytes memory initcode =
            hex"6040600b3d3960403df3fe36383d373d3d6020515160208051013d3d515af160203851516084018038385101606037303452813582523838523490601c34355afa34513060e01b141634fd";
        (bool ok,) = 0x4e59b44847b379578588920cA78FbF26c0B4956C
        .call(abi.encodePacked(bytes32(0x000000000000000000000000000000000000000068f35e1510740001fd13984a), initcode));
        require(ok, "verifier");
        MockAccountFactory factory = new MockAccountFactory();
        address account = factory.predict(seller, bytes32(uint256(7)));
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.seller = account;
        bytes memory sig = abi.encodePacked(
            abi.encode(
                address(factory),
                abi.encodeCall(MockAccountFactory.deploy, (seller, bytes32(uint256(7)))),
                _ssign(o, sellerKey)
            ),
            bytes32(0x6492649264926492649264926492649264926492649264926492649264926492)
        );
        vm.prank(buyer);
        uint256 id = stream.createAndFund(o, sig);
        assertEq(_order(id).seller, account);
    }

    function test_revert_badTerms() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);

        o.seller = buyer; // self-dealing, even with the buyer's own valid signature
        (, uint256 buyerKey) = makeAddrAndKey("buyer");
        bytes memory sig = _ssign(o, buyerKey);
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.BadOffer.selector);
        stream.createAndFund(o, sig);
        o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.amount = 4 * U;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.amount = 201 * U;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.amount = 100 * U;
        o.bond = 101 * U;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.lateSlashBps = 10_001; // two failed checks at once
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.bond = 0;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.lateSlashBps = 0;
        o.startWindow = 1 days - 1;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.startWindow = 7 days + 1;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.startWindow = 3 days;
        o.periods = 0;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.interval = StreamEscrow.Interval.Month;
        o.periods = 13; // 390 days
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.interval = StreamEscrow.Interval.Week;
        o.periods = 53;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.interval = StreamEscrow.Interval.Hour;
        o.periods = 8761;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
    }

    function test_createAndFund_startWindowBounds() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.startWindow = 1 days;
        _screate(o);
        o.startWindow = 7 days;
        o.nonce = 2;
        _screate(o);
    }

    function test_createAndFund_longestDurations() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.interval = StreamEscrow.Interval.Month;
        o.periods = 12; // 360 days
        uint256 id = _screate(o);
        assertEq(_order(id).interval, 30 days);
        o.interval = StreamEscrow.Interval.Week;
        o.periods = 52;
        o.nonce = 2;
        id = _screate(o);
        assertEq(_order(id).interval, 7 days);
        o.interval = StreamEscrow.Interval.Hour;
        o.periods = 989; // 98.9 USDT / 989 = exactly 0.1 USDT per period
        o.nonce = 3;
        id = _screate(o);
        assertEq(_order(id).interval, 1 hours);
    }

    /// Plan §6: each period must unlock at least 0.1 USDT of net principal.
    function test_revert_perPeriodBelowMinimum() public {
        StreamEscrow.Offer memory o = _soffer(5 * U, FeeMath.Payer.Seller); // net 3.9 USDT
        o.interval = StreamEscrow.Interval.Hour;
        o.periods = 40;
        _expectCreateRevert(o, StreamEscrow.BadOffer.selector);
        o.periods = 39;
        assertEq(_order(_screate(o)).net, 3_900_000);
    }

    function test_revert_feeLimits() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.maxFeeBps = 99;
        _expectCreateRevert(o, StreamEscrow.FeeLimit.selector);
        o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.maxBaseGas = 99_999;
        _expectCreateRevert(o, StreamEscrow.FeeLimit.selector);
        o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.maxClaimGas = 19_999;
        _expectCreateRevert(o, StreamEscrow.FeeLimit.selector);
        o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.maxMinFee = 999_999;
        _expectCreateRevert(o, StreamEscrow.FeeLimit.selector);
        o.maxClaimGas = 19_999; // two limits exceeded at once
        _expectCreateRevert(o, StreamEscrow.FeeLimit.selector);
    }

    function test_revert_feesExceedAmount() public {
        EscrowConfig.Params memory p = defaultParams();
        p.minOrder = uint64(1 * U);
        vm.prank(owner);
        config.setParams(p); // loosening: immediate
        // 1.1 USDT: seller-borne min fee 1 + gas 0.1 == amount leaves no principal to stream
        _expectCreateRevert(_soffer(1_100_000, FeeMath.Payer.Seller), StreamEscrow.FeesExceedAmount.selector);
    }

    function test_revert_configGates() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.token = address(0xbeef);
        _expectCreateRevert(o, EscrowConfig.TokenNotAllowed.selector);

        vm.prank(owner);
        config.setPaused(true);
        _expectCreateRevert(_soffer(100 * U, FeeMath.Payer.Seller), EscrowConfig.IsPaused.selector);
        vm.prank(owner);
        config.setPaused(false);

        EscrowConfig.Params memory p = defaultParams();
        p.totalLockedCap = uint64(200 * U);
        vm.prank(owner);
        config.setParams(p);
        o = _soffer(100 * U, FeeMath.Payer.Seller);
        _screate(o);
        o.nonce = 2;
        _screate(o);
        o.nonce = 3;
        _expectCreateRevert(o, EscrowConfig.TotalLockedCap.selector);
    }

    function test_revert_feeOnTransferToken() public {
        FeeOnTransferToken fot = new FeeOnTransferToken();
        (,, StreamEscrow s) = _deployBoth(address(fot), defaultParams());
        fot.mint(buyer, 1000 * U);
        vm.prank(buyer);
        fot.approve(address(s), type(uint256).max);
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.token = address(fot);
        (uint8 v, bytes32 r, bytes32 sg) = vm.sign(sellerKey, s.hashOffer(o));
        vm.prank(buyer);
        vm.expectRevert(MarketBase.AmountMismatch.selector);
        s.createAndFund(o, abi.encodePacked(r, sg, v));
    }

    function test_createAndFund_locksBond() public {
        _depositBond(60 * U);
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.bond = 50 * U;
        uint256 id = _screate(o);
        assertEq(_order(id).bond, 50 * U);
        (uint256 total, uint256 locked,) = _bond();
        assertEq(total, 60 * U);
        assertEq(locked, 50 * U);
        assertEq(escrow.lockedBy(address(stream), seller, address(usdt)), 50 * U);
        assertEq(config.totalLocked(), 160 * U);

        o.nonce = 2;
        _expectCreateRevert(o, Escrow.InsufficientBond.selector);
    }

    function test_createAndFund_gasSubsidyWaivesClaimGas() public {
        uint8 scope = config.SCOPE_ESCROW();
        vm.prank(owner);
        config.setCampaign(
            EscrowConfig.Campaign({
                start: uint64(block.timestamp),
                end: uint64(block.timestamp + 10 days),
                rateBps: 0,
                scope: scope,
                gasSubsidyCap: 1
            })
        );
        uint256 id = _screateDefault();
        StreamEscrow.Order memory o = _order(id);
        assertEq(o.sellerFee, 0);
        assertEq(o.sellerGas, 0);
        assertEq(o.claimGas, 0);
        assertEq(o.net, 100 * U);
        assertEq(o.disputeFee, 1 * U); // floor: the regular minimum fee

        _start(id);
        vm.warp(block.timestamp + 1 days);
        _claim(id);
        assertEq(_bal(feeRecipient), 0);
        assertEq(_order(id).claimed, 10 * U);
    }

    function test_createAndFund_snapshotSurvivesParamChange() public {
        uint256 id = _screateDefault();
        EscrowConfig.Params memory p = defaultParams();
        p.claimGas = 0;
        vm.prank(owner);
        config.setParams(p); // loosening: immediate
        assertEq(config.params().claimGas, 0);
        assertEq(_order(id).claimGas, CLAIM_GAS);
    }

    function test_intervalSeconds() public view {
        assertEq(stream.intervalSeconds(StreamEscrow.Interval.Hour), 1 hours);
        assertEq(stream.intervalSeconds(StreamEscrow.Interval.Day), 1 days);
        assertEq(stream.intervalSeconds(StreamEscrow.Interval.Week), 7 days);
        assertEq(stream.intervalSeconds(StreamEscrow.Interval.Month), 30 days);
    }
}

contract StreamLifecycleTest is StreamBase {
    uint256 internal id;

    function setUp() public override {
        super.setUp();
        id = _screateDefault();
    }

    // ── start ──

    function test_start() public {
        vm.expectEmit(address(stream));
        emit StreamEscrow.Started(id, uint64(block.timestamp), uint64(block.timestamp + 10 days));
        _start(id);
        assertEq(_order(id).startedAt, block.timestamp);
        assertTrue(_order(id).platformPaid);
    }

    function test_revert_start() public {
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.start(id);

        vm.warp(block.timestamp + 3 days + 1);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.TooLate.selector);
        stream.start(id);
    }

    function test_revert_startTwice() public {
        _start(id);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.start(id);
    }

    function test_revert_unknownOrder() public {
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.start(99);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.proposeEnd(99, 0);
    }

    /// Access checks compare addresses for equality only: callers just below and above every party are rejected.
    function test_revert_notParty_anyAddress() public {
        address[2] memory others = [address(1), address(type(uint160).max)];
        uint256[] memory ids = new uint256[](1);
        ids[0] = id;
        for (uint256 i; i < 2; ++i) {
            vm.startPrank(others[i]);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.start(id);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.claim(id);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.refund(id);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.proposeEnd(id, 0);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.acceptEnd(id, 0);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.dispute(id);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.disputeWithBond(id);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.resolve(id, 0, 0);
            vm.expectRevert(StreamEscrow.NotParty.selector);
            stream.extendDeadlines(ids, 1 days);
            vm.stopPrank();
        }
    }

    /// Orders without a bond never touch the bond pool (no BondLocked / BondReleased for the indexer to misread).
    function test_noBond_noPoolCalls() public {
        vm.recordLogs();
        uint256 b = _screate(_nextOffer(2));
        vm.prank(seller);
        stream.refund(b);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        for (uint256 i; i < logs.length; ++i) {
            assertTrue(logs[i].emitter != address(escrow), "no bond pool event");
        }
    }

    // ── unlock & claim ──

    function test_unlockSchedule() public {
        vm.warp(block.timestamp + 2 days);
        assertEq(stream.unlocked(id), 0, "nothing unlocks before start");
        _start(id);
        uint256 t0 = block.timestamp;
        vm.warp(t0 + 1 days - 1);
        assertEq(stream.unlocked(id), 0);
        vm.warp(t0 + 1 days);
        assertEq(stream.unlocked(id), PER_DAY);
        vm.warp(t0 + 9 days + 23 hours);
        assertEq(stream.unlocked(id), 9 * PER_DAY);
        vm.warp(t0 + 10 days);
        assertEq(stream.unlocked(id), NET);
        vm.warp(t0 + 400 days);
        assertEq(stream.unlocked(id), NET);
    }

    function test_claim() public {
        _start(id);
        vm.warp(block.timestamp + 2 days + 5 hours);
        uint256 s0 = _bal(seller);
        vm.expectEmit(address(stream));
        emit StreamEscrow.Claimed(id, 2 * PER_DAY - CLAIM_GAS, CLAIM_GAS);
        _claim(id);
        assertEq(_bal(seller) - s0, 2 * PER_DAY - CLAIM_GAS);
        assertEq(_bal(feeRecipient), CLAIM_GAS);
        assertEq(_order(id).claimed, 2 * PER_DAY);
        assertEq(_order(id).lastClaimAt, block.timestamp);
        assertEq(config.totalLocked(), 100 * U - 2 * PER_DAY);
        assertEq(_bal(address(stream)), 100 * U - 2 * PER_DAY);
    }

    function test_revert_claim() public {
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.claim(id);

        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector); // not started
        stream.claim(id);

        _start(id);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.NothingToClaim.selector); // nothing unlocked yet
        stream.claim(id);
    }

    /// Two claims are at least 24 hours apart, so each claim-gas fee matches one sponsored daily claim.
    function test_claim_24hBetweenClaims() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.interval = StreamEscrow.Interval.Hour;
        o.periods = 72;
        o.nonce = 2;
        uint256 h = _screate(o);
        _start(h);
        uint256 t0 = block.timestamp;
        vm.warp(t0 + 1 hours);
        _claim(h);
        vm.warp(t0 + 1 hours + 24 hours - 1);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.TooEarly.selector);
        stream.claim(h);
        vm.warp(t0 + 1 hours + 24 hours);
        _claim(h);
        assertEq(_order(h).claimed, uint256(NET) * 25 / 72);
        assertEq(_bal(feeRecipient), 2 * CLAIM_GAS);
        vm.warp(t0 + 1 hours + 24 hours + 25 hours); // any time after the 24 hours, not only at the mark
        _claim(h);
        assertEq(_order(h).claimed, uint256(NET) * 50 / 72);
        assertEq(_bal(feeRecipient), 3 * CLAIM_GAS);
    }

    function test_revert_claim_notMoreThanClaimGas() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.interval = StreamEscrow.Interval.Week;
        o.periods = 4;
        o.nonce = 2;
        uint256 w = _screate(o);
        _start(w);
        vm.warp(block.timestamp + 7 days);
        _claim(w);
        vm.warp(block.timestamp + 1 days); // 24h passed but nothing new unlocked
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.NothingToClaim.selector);
        stream.claim(w);
    }

    function test_claim_afterEndSettles() public {
        _start(id);
        vm.warp(block.timestamp + 3 days);
        _claim(id);
        uint256 late = _screate(_nextOffer(2));
        _start(late);
        vm.warp(block.timestamp + 7 days);
        uint256 s0 = _bal(seller);
        _claim(id); // stream over: settles, no claim gas
        assertEq(uint8(_order(id).state), uint8(StreamEscrow.State.Released));
        assertEq(_bal(seller) - s0, NET - 3 * PER_DAY);
        assertEq(_bal(feeRecipient), CLAIM_GAS + 1_100_000);
        vm.warp(block.timestamp + 4 days); // well after the end, too
        _claim(late);
        assertEq(uint8(_order(late).state), uint8(StreamEscrow.State.Released));
        assertEq(_bal(address(stream)), 0);
        assertEq(config.totalLocked(), 0);
    }

    function test_claim_frozenSellerCredited() public {
        _start(id);
        vm.warp(block.timestamp + 1 days);
        usdt.setFrozen(seller, true);
        _claim(id);
        assertEq(stream.credit(seller, address(usdt)), PER_DAY - CLAIM_GAS);
        vm.prank(seller);
        stream.withdraw(address(usdt), stranger);
        assertEq(_bal(stranger), PER_DAY - CLAIM_GAS);
        assertEq(stream.credit(seller, address(usdt)), 0);

        vm.prank(seller);
        vm.expectRevert(MarketBase.NothingToWithdraw.selector);
        stream.withdraw(address(usdt), stranger);
    }

    // ── finalize ──

    function test_finalize() public {
        _start(id);
        vm.warp(block.timestamp + 10 days - 1);
        vm.expectRevert(StreamEscrow.TooEarly.selector);
        stream.finalize(id);
        uint256 atEnd = _screate(_nextOffer(2));
        _start(atEnd);
        vm.warp(block.timestamp + 10 days);
        stream.finalize(atEnd); // exactly at the end
        vm.warp(block.timestamp + 1 days); // and any time later
        uint256 s0 = _bal(seller);
        vm.prank(stranger); // the platform worker
        stream.finalize(id);
        assertEq(uint8(_order(id).state), uint8(StreamEscrow.State.Released));
        assertEq(_bal(seller) - s0, NET);
        assertEq(_bal(feeRecipient), 2 * 1_100_000);
        assertEq(_bal(address(stream)), 0);

        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.finalize(id);
    }

    function test_revert_finalize_notStarted() public {
        vm.warp(block.timestamp + 365 days);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.finalize(id);
    }

    function test_finalize_releasesBond() public {
        _depositBond(50 * U);
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.bond = 50 * U;
        o.nonce = 2;
        uint256 b = _screate(o);
        _start(b);
        vm.warp(block.timestamp + 10 days);
        stream.finalize(b);
        (uint256 total, uint256 locked,) = _bond();
        assertEq(total, 50 * U);
        assertEq(locked, 0);
        assertEq(escrow.lockedBy(address(stream), seller, address(usdt)), 0);
    }

    // ── refund / claimTimeout ──

    function test_refund_beforeStart_full() public {
        uint256 b0 = _bal(buyer);
        vm.prank(seller);
        stream.refund(id);
        assertEq(uint8(_order(id).state), uint8(StreamEscrow.State.Refunded));
        assertEq(_bal(buyer) - b0, 100 * U);
        assertEq(_bal(feeRecipient), 0);
        assertEq(config.totalLocked(), 0);
    }

    /// After a claim, fees are charged only on the claimed share; base gas is earned (the platform sponsored start).
    function test_refund_afterClaim() public {
        _start(id);
        vm.warp(block.timestamp + 1 days);
        _claim(id);
        uint256 b0 = _bal(buyer);
        vm.prank(seller);
        stream.refund(id);
        assertEq(_bal(buyer) - b0, 89_910_000);
        assertEq(_bal(feeRecipient), CLAIM_GAS + 200_000); // claim gas + fee on 1/10 + base gas
        assertEq(_bal(address(stream)), 0);
    }

    function test_revert_refund() public {
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.refund(id);
    }

    function test_claimTimeout_slashesBond() public {
        _depositBond(50 * U);
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.bond = 40 * U;
        o.lateSlashBps = 5000;
        o.nonce = 2;
        uint256 b = _screate(o);
        vm.expectRevert(StreamEscrow.TooEarly.selector);
        stream.claimTimeout(b);

        vm.warp(block.timestamp + 3 days);
        vm.expectRevert(StreamEscrow.TooEarly.selector);
        stream.claimTimeout(b);

        vm.warp(block.timestamp + 1);
        uint256 b0 = _bal(buyer);
        vm.prank(stranger);
        stream.claimTimeout(b);
        assertEq(uint8(_order(b).state), uint8(StreamEscrow.State.Refunded));
        assertEq(_bal(buyer) - b0, 120 * U);
        (uint256 total, uint256 locked,) = _bond();
        assertEq(total, 30 * U);
        assertEq(locked, 0);
        assertEq(config.totalLocked(), 100 * U + 30 * U); // the untouched default order + remaining bond
    }

    function test_revert_claimTimeout_started() public {
        _start(id);
        vm.warp(block.timestamp + 30 days);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.claimTimeout(id);
    }

    // ── extendDeadlines ──

    function test_extendDeadlines() public {
        uint256 started = _screate(_nextOffer(2));
        _start(started);
        uint256[] memory ids = new uint256[](3);
        (ids[0], ids[1], ids[2]) = (id, started, 99);
        uint64 d0 = _order(id).startDeadline;

        vm.prank(arbiter);
        stream.extendDeadlines(ids, 7 days);
        assertEq(_order(id).startDeadline, d0 + 7 days);
        assertEq(_order(id).arbExtended, 7 days);
        assertEq(_order(started).arbExtended, 0, "started orders have no start deadline");
        vm.prank(arbiter);
        stream.extendDeadlines(ids, 7 days);
        vm.prank(arbiter);
        stream.extendDeadlines(ids, 1); // over the 14-day cap: skipped
        assertEq(_order(id).startDeadline, d0 + 14 days);

        vm.warp(d0 + 14 days);
        _start(id); // still in time
        vm.prank(arbiter);
        stream.extendDeadlines(ids, 1); // started and over the cap: still skipped
        assertEq(_order(id).startDeadline, d0 + 14 days);
    }

    function test_revert_extendDeadlines() public {
        uint256[] memory ids = new uint256[](1);
        ids[0] = id;
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.extendDeadlines(ids, 1 days);
        vm.startPrank(arbiter);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        stream.extendDeadlines(ids, 0);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        stream.extendDeadlines(ids, 7 days + 1);
        vm.stopPrank();
    }

    // ── proposeEnd / acceptEnd ──

    function test_acceptEnd_defaultSuggestion() public {
        _start(id);
        vm.warp(block.timestamp + 4 days);
        uint256 toBuyer = NET - stream.unlocked(id);
        assertEq(toBuyer, 59_340_000);
        vm.expectEmit(address(stream));
        emit StreamEscrow.EndProposed(id, buyer, toBuyer, uint64(block.timestamp + 72 hours));
        vm.prank(buyer);
        stream.proposeEnd(id, toBuyer);
        uint256 b0 = _bal(buyer);
        uint256 s0 = _bal(seller);
        vm.prank(seller);
        stream.acceptEnd(id, toBuyer);
        assertEq(uint8(_order(id).state), uint8(StreamEscrow.State.Settled));
        assertEq(_bal(seller) - s0, 39_560_000);
        assertEq(_bal(buyer) - b0, 59_940_000); // + unearned 60% of the 1 USDT fee
        assertEq(_bal(feeRecipient), 500_000);
    }

    function test_acceptEnd_atProposalExpiry() public {
        vm.prank(buyer);
        stream.proposeEnd(id, NET);
        vm.warp(_order(id).proposalExpiry);
        vm.prank(seller);
        stream.acceptEnd(id, NET);
    }

    /// A proposal is a sponsored operation: the base gas is earned even if nobody accepts and the seller refunds.
    function test_proposeEnd_earnsGas() public {
        vm.prank(buyer);
        stream.proposeEnd(id, NET);
        vm.prank(seller);
        stream.refund(id);
        assertEq(_bal(feeRecipient), 100_000);
    }

    function test_acceptEnd_beforeStart_earnsGas() public {
        vm.prank(seller);
        stream.proposeEnd(id, NET);
        uint256 b0 = _bal(buyer);
        vm.prank(buyer);
        stream.acceptEnd(id, NET);
        assertEq(_bal(buyer) - b0, 100 * U - 100_000);
        assertEq(_bal(feeRecipient), 100_000);
    }

    function test_revert_proposeEnd() public {
        vm.prank(stranger);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.proposeEnd(id, 0);
        _start(id);
        vm.warp(block.timestamp + 1 days);
        _claim(id);
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.BadParams.selector); // more than the unclaimed principal
        stream.proposeEnd(id, NET - PER_DAY + 1);
        vm.prank(buyer);
        stream.proposeEnd(id, NET - PER_DAY); // all of the unclaimed principal is fine
    }

    function test_revert_acceptEnd() public {
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.BadState.selector); // no proposal
        stream.acceptEnd(id, 0);

        vm.prank(buyer);
        stream.proposeEnd(id, NET);
        vm.prank(stranger);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.acceptEnd(id, NET);
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.BadState.selector); // own proposal
        stream.acceptEnd(id, NET);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadParams.selector); // swapped amount
        stream.acceptEnd(id, NET - 1);
        vm.warp(block.timestamp + 72 hours + 1);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.TooLate.selector);
        stream.acceptEnd(id, NET);
    }

    /// A claim made after the proposal can make it unpayable; accepting then reverts instead of underflowing.
    function test_revert_acceptEnd_afterClaimShrankUnclaimed() public {
        _start(id);
        vm.prank(buyer);
        stream.proposeEnd(id, NET);
        vm.warp(block.timestamp + 1 days);
        _claim(id);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        stream.acceptEnd(id, NET);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadParams.selector); // neither the proposal nor payable
        stream.acceptEnd(id, NET + 1);
    }

    function test_revert_settledOrderIsFinal() public {
        vm.prank(seller);
        stream.refund(id);
        vm.startPrank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.proposeEnd(id, 0);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.dispute(id);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.refund(id);
        vm.stopPrank();
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.claimTimeout(id);
        vm.prank(arbiter);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.resolve(id, 0, 0);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.resolveTimeout(id);
    }

    function _nextOffer(uint256 nonce) internal view returns (StreamEscrow.Offer memory o) {
        o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.nonce = nonce;
    }
}

contract StreamDisputeTest is StreamBase {
    uint256 internal id;

    function setUp() public override {
        super.setUp();
        id = _screateDefault();
    }

    function _dispute(address who) internal {
        vm.prank(who);
        stream.dispute(id);
    }

    function test_dispute_buyer_freezesUnlock() public {
        _start(id);
        vm.warp(block.timestamp + 3 days + 12 hours);
        uint256 b0 = _bal(buyer);
        vm.expectEmit(address(stream));
        emit StreamEscrow.Disputed(id, buyer, DISPUTE_FEE, StreamEscrow.FeeSource.Wallet);
        _dispute(buyer);
        assertEq(b0 - _bal(buyer), DISPUTE_FEE);
        StreamEscrow.Order memory o = _order(id);
        assertEq(uint8(o.state), uint8(StreamEscrow.State.Disputed));
        assertEq(o.disputedAt, block.timestamp);
        assertFalse(o.sellerDisputed);
        assertEq(config.totalLocked(), 101 * U);

        assertEq(stream.unlocked(id), 3 * PER_DAY);
        vm.warp(block.timestamp + 20 days);
        assertEq(stream.unlocked(id), 3 * PER_DAY, "no unlock growth after dispute");
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.claim(id);
    }

    function test_dispute_seller_paysFromUnlocked() public {
        _start(id);
        vm.warp(block.timestamp + 3 days);
        uint256 s0 = _bal(seller);
        vm.expectEmit(address(stream));
        emit StreamEscrow.Disputed(id, seller, DISPUTE_FEE, StreamEscrow.FeeSource.Unlocked);
        _dispute(seller);
        assertEq(_bal(seller), s0, "no wallet payment");
        assertEq(_order(id).claimed, DISPUTE_FEE);
        assertEq(config.totalLocked(), 100 * U);

        // Seller wins: everything unclaimed + the dispute fee back.
        uint256 s1 = _bal(seller);
        vm.prank(arbiter);
        stream.resolve(id, 0, 0);
        assertEq(_bal(seller) - s1, NET);
        assertEq(_bal(feeRecipient), 1_100_000);
        assertEq(_bal(address(stream)), 0);
    }

    /// Unlocked principal exactly equal to the dispute fee is enough to pay it.
    function test_dispute_seller_paysFromUnlocked_exactly() public {
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Buyer); // net 100, 100 days: 1 USDT a day
        o.periods = 100;
        o.nonce = 2;
        uint256 b = _screate(o);
        _start(b);
        vm.warp(block.timestamp + 1 days);
        assertEq(stream.unlocked(b), _order(b).disputeFee);
        vm.prank(seller);
        stream.dispute(b);
        assertEq(uint8(_order(b).disputeSource), uint8(StreamEscrow.FeeSource.Unlocked));
    }

    function test_dispute_seller_walletWhenUnlockedTooSmall() public {
        uint256 s0 = _bal(seller);
        _dispute(seller); // not started: nothing unlocked
        assertEq(s0 - _bal(seller), DISPUTE_FEE);
        assertEq(uint8(_order(id).disputeSource), uint8(StreamEscrow.FeeSource.Wallet));
        assertTrue(_order(id).sellerDisputed);
    }

    function test_disputeWithBond() public {
        _depositBond(10 * U);
        uint256 lockedBefore = config.totalLocked();
        vm.prank(seller);
        stream.disputeWithBond(id);
        assertEq(uint8(_order(id).disputeSource), uint8(StreamEscrow.FeeSource.Bond));
        (uint256 total,,) = _bond();
        assertEq(total, 9 * U);
        assertEq(_bal(address(stream)), 101 * U);
        assertEq(config.totalLocked(), lockedBefore, "moved between markets, custody unchanged");

        // Seller loses fully: the dispute fee is retained, buyer refunded in full (not started: no gas earned).
        uint256 b0 = _bal(buyer);
        vm.prank(arbiter);
        stream.resolve(id, 10_000, 0);
        assertEq(_bal(buyer) - b0, 100 * U);
        assertEq(_bal(feeRecipient), DISPUTE_FEE);
    }

    function test_disputeWithBond_refundGoesToSellerWallet() public {
        _depositBond(10 * U);
        vm.prank(seller);
        stream.disputeWithBond(id);
        vm.warp(block.timestamp + 30 days + 1);
        uint256 s0 = _bal(seller);
        stream.resolveTimeout(id);
        assertEq(_bal(seller) - s0, DISPUTE_FEE);
    }

    function test_revert_disputeWithBond() public {
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.disputeWithBond(id);
        vm.prank(seller);
        vm.expectRevert(Escrow.InsufficientBond.selector);
        stream.disputeWithBond(id);
    }

    function test_dispute_atStartDeadline() public {
        vm.warp(_order(id).startDeadline);
        _dispute(buyer);
    }

    function test_revert_dispute() public {
        vm.prank(stranger);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.dispute(id);

        vm.warp(block.timestamp + 3 days + 1); // start deadline missed: claimTimeout, not dispute
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.TooLate.selector);
        stream.dispute(id);
    }

    function test_revert_dispute_afterEnd() public {
        _start(id);
        vm.warp(block.timestamp + 10 days);
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.TooLate.selector);
        stream.dispute(id);
        vm.warp(block.timestamp + 1 days);
        vm.prank(buyer);
        vm.expectRevert(StreamEscrow.TooLate.selector);
        stream.dispute(id);
    }

    function test_revert_disputeTwice() public {
        _dispute(buyer);
        vm.prank(seller);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.dispute(id);
    }

    /// The ruling covers everything unclaimed, including principal unlocked before the dispute.
    function test_resolve_splitsAllUnclaimed() public {
        _start(id);
        vm.warp(block.timestamp + 1 days);
        _claim(id);
        vm.warp(block.timestamp + 2 days + 12 hours);
        _dispute(buyer);
        uint256 b0 = _bal(buyer);
        uint256 s0 = _bal(seller);
        vm.expectEmit(address(stream));
        emit StreamEscrow.Resolved(id, 5000, 0);
        vm.prank(arbiter);
        stream.resolve(id, 5000, 0);
        assertEq(uint8(_order(id).state), uint8(StreamEscrow.State.Settled));
        assertEq(_bal(seller) - s0, 44_505_000);
        assertEq(_bal(buyer) - b0, 45_955_000); // incl. half the dispute fee back
        // claim gas + base gas + max(0, fee on 55% - retained 0.5)
        assertEq(_bal(feeRecipient), CLAIM_GAS + 100_000 + 50_000 + 500_000);
        assertEq(_bal(address(stream)), 0);
        assertEq(config.totalLocked(), 0);
    }

    /// Seller disputes and mostly loses: the retained 3/4 of the dispute fee exceeds the fee earned on the seller's
    /// quarter, so the platform keeps base gas + the retained part and charges no platform fee.
    function test_resolve_sellerDisputerMostlyLoses() public {
        _start(id);
        vm.warp(block.timestamp + 1 days);
        vm.prank(seller);
        stream.dispute(id); // paid from the unlocked first day
        vm.prank(arbiter);
        stream.resolve(id, 7500, 0);
        assertEq(_bal(feeRecipient), 100_000 + 750_000);
        assertEq(_bal(address(stream)), 0);
    }

    function test_resolve_slashesBond() public {
        _depositBond(50 * U);
        StreamEscrow.Offer memory o = _soffer(100 * U, FeeMath.Payer.Seller);
        o.bond = 50 * U;
        o.nonce = 2;
        uint256 b = _screate(o);
        vm.prank(buyer);
        stream.dispute(b);
        uint256 b0 = _bal(buyer);
        vm.prank(arbiter);
        stream.resolve(b, 10_000, 2000);
        assertEq(_bal(buyer) - b0, 100 * U + DISPUTE_FEE + 10 * U);
        (uint256 total, uint256 locked,) = _bond();
        assertEq(total, 40 * U);
        assertEq(locked, 0);
    }

    function test_revert_resolve() public {
        vm.prank(arbiter);
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.resolve(id, 0, 0);
        _dispute(buyer);
        vm.expectRevert(StreamEscrow.NotParty.selector);
        stream.resolve(id, 0, 0);
        vm.startPrank(arbiter);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        stream.resolve(id, 10_001, 0);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        stream.resolve(id, 0, 10_001);
        vm.expectRevert(StreamEscrow.BadParams.selector);
        stream.resolve(id, 10_001, 10_001);
        vm.stopPrank();
    }

    function test_resolveTimeout_notStarted_fullRefund() public {
        _dispute(buyer);
        vm.expectRevert(StreamEscrow.TooEarly.selector);
        stream.resolveTimeout(id);
        vm.warp(block.timestamp + 30 days);
        vm.expectRevert(StreamEscrow.TooEarly.selector);
        stream.resolveTimeout(id);
        vm.warp(block.timestamp + 1);
        uint256 b0 = _bal(buyer);
        stream.resolveTimeout(id);
        assertEq(_bal(buyer) - b0, 100 * U + DISPUTE_FEE);
        assertEq(_bal(feeRecipient), 0);
    }

    /// Plan §6: seller keeps what had unlocked at the dispute, the rest of the unclaimed is split 50/50.
    function test_resolveTimeout_started() public {
        _start(id);
        vm.warp(block.timestamp + 3 days + 12 hours);
        _dispute(buyer);
        vm.warp(block.timestamp + 30 days + 1);
        uint256 b0 = _bal(buyer);
        uint256 s0 = _bal(seller);
        stream.resolveTimeout(id);
        assertEq(_bal(seller) - s0, 64_285_000);
        assertEq(_bal(buyer) - b0, 35_965_000);
        assertEq(_bal(feeRecipient), 750_000);
    }

    function test_revert_resolveTimeout_notDisputed() public {
        vm.expectRevert(StreamEscrow.BadState.selector);
        stream.resolveTimeout(id);
    }

    function test_acceptEnd_duringDispute_refundsDisputeFee() public {
        _start(id);
        vm.warp(block.timestamp + 2 days);
        _dispute(buyer);
        vm.prank(seller);
        stream.proposeEnd(id, NET / 2);
        uint256 b0 = _bal(buyer);
        vm.prank(buyer);
        stream.acceptEnd(id, NET / 2);
        // half the principal + half the seller fee (unearned) + the dispute fee
        assertEq(_bal(buyer) - b0, NET / 2 + 500_000 + DISPUTE_FEE);
        assertEq(_bal(feeRecipient), 600_000);
    }

    function test_settle_frozenBuyerCredited() public {
        _dispute(buyer);
        usdt.setFrozen(buyer, true);
        vm.prank(arbiter);
        stream.resolve(id, 10_000, 0);
        assertEq(stream.credit(buyer, address(usdt)), 100 * U + DISPUTE_FEE);
        usdt.setFrozen(buyer, false);
        vm.prank(buyer);
        stream.withdraw(address(usdt), buyer);
        assertEq(_bal(address(stream)), 0);
    }
}
