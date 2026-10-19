// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Escrow} from "../src/Escrow.sol";
import {MarketBase} from "../src/MarketBase.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FeeOnTransferToken} from "./mocks/TestTokens.sol";
import {EscrowBase} from "./EscrowBase.sol";

contract EscrowCreateTest is EscrowBase {
    function test_createAndFund_sellerPays() public {
        uint256 b0 = _bal(buyer);
        uint256 id = _createDefault();
        assertEq(id, 1);
        assertEq(b0 - _bal(buyer), 100 * U);
        assertEq(_bal(address(escrow)), 100 * U);
        Escrow.Order memory o = escrow.getOrder(id);
        assertEq(uint8(o.state), uint8(Escrow.State.Funded));
        assertEq(o.buyer, buyer);
        assertEq(o.seller, seller);
        assertEq(o.sellerFee, 1 * U);
        assertEq(o.buyerFee, 0);
        assertEq(o.sellerGas, 100_000);
        assertEq(o.disputeFee, 1 * U);
        assertEq(o.shipDeadline, block.timestamp + 3 days);
        assertEq(o.confirmDeadline, 0);
        assertEq(o.resolveTimeout, 30 days);
        assertEq(o.proposalWindow, 72 hours);
        assertFalse(o.platformPaid);
    }

    /// The digest must equal the textbook EIP-712 encoding, so viem / alloy signers produce the same hash.
    function test_hashOffer_matchesEip712FieldEncoding() public view {
        Escrow.Offer memory o = _offer(123 * U, FeeMath.Payer.Half);
        o.buyer = buyer;
        o.instant = true;
        o.bond = 7 * U;
        o.lateSlashBps = 2500;
        bytes memory fields = abi.encode(
            o.seller, o.buyer, o.token, o.amount, uint8(o.feePayer), o.maxFeeBps, o.maxBaseGas, o.maxMinFee, o.bond
        );
        bytes32 structHash = keccak256(
            abi.encodePacked(
                escrow.OFFER_TYPEHASH(),
                fields,
                abi.encode(o.lateSlashBps, o.instant, o.shipWindow),
                abi.encode(o.confirmWindow, o.listingHash, o.expiry, o.maxFills, o.nonce)
            )
        );
        bytes32 domain = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("AkariEscrow"),
                keccak256("1"),
                block.chainid,
                address(escrow)
            )
        );
        assertEq(escrow.hashOffer(o), keccak256(abi.encodePacked("\x19\x01", domain, structHash)));
    }

    function test_createAndFund_buyerAndHalf() public {
        uint256 b0 = _bal(buyer);
        _create(_offer(100 * U, FeeMath.Payer.Buyer));
        assertEq(b0 - _bal(buyer), 101_100_000);
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Half);
        o.nonce = 2;
        b0 = _bal(buyer);
        uint256 id = _create(o);
        assertEq(b0 - _bal(buyer), 100_550_000);
        assertEq(escrow.getOrder(id).sellerFee, 500_000);
    }

    function test_createAndFund_instantStartsConfirm() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.instant = true;
        o.shipWindow = 0; // ignored for instant delivery
        uint256 id = _create(o);
        assertEq(escrow.getOrder(id).confirmDeadline, block.timestamp + 7 days);
        assertEq(escrow.getOrder(id).shipDeadline, 0);
    }

    function test_createAndFund_designatedBuyer() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.buyer = buyer;
        _create(o);
        o.nonce = 2;
        o.buyer = stranger;
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadOffer.selector);
        escrow.createAndFund(o, sig);
    }

    function test_createAndFund_maxFills() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.maxFills = 2;
        _create(o);
        _create(o);
        assertEq(escrow.fills(escrow.hashOffer(o)), 2);
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(MarketBase.OfferUnavailable.selector);
        escrow.createAndFund(o, sig);
    }

    function test_createAndFund_snapshotSurvivesParamChange() public {
        uint256 id = _createDefault();
        EscrowConfig.Params memory p = defaultParams();
        p.feeBps = 0;
        p.minFee = 0;
        vm.prank(owner);
        config.setParams(p); // loosening: immediate
        assertEq(config.params().feeBps, 0);
        assertEq(escrow.getOrder(id).sellerFee, 1 * U);
    }

    function _startCampaign(uint16 rateBps, uint32 subsidy) internal {
        vm.prank(owner);
        config.setCampaign(
            EscrowConfig.Campaign({
                start: uint64(block.timestamp),
                end: uint64(block.timestamp + 30 days),
                rateBps: rateBps,
                scope: 3,
                gasSubsidyCap: subsidy
            })
        );
    }

    function test_createAndFund_campaign() public {
        _startCampaign(0, 0);
        uint256 id = _createDefault();
        Escrow.Order memory o = escrow.getOrder(id);
        assertEq(o.sellerFee, 0);
        assertEq(o.disputeFee, 1 * U); // zero-fee order: dispute fee based on min fee
    }

    function _expectCreateRevert(Escrow.Offer memory o, bytes4 err) internal {
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(err);
        escrow.createAndFund(o, sig);
    }

    function test_revert_create_offerChecks() public {
        Escrow.Offer memory o;

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.seller = address(0); // no key can sign for the zero address
        _expectCreateRevert(o, MarketBase.BadSignature.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller); // seller buying their own offer
        bytes memory own = _sign(o, sellerKey);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadOffer.selector);
        escrow.createAndFund(o, own);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.expiry = uint64(block.timestamp - 1);
        _expectCreateRevert(o, MarketBase.OfferUnavailable.selector);

        o = _offer(4 * U, FeeMath.Payer.Seller);
        _expectCreateRevert(o, Escrow.BadOffer.selector);

        o = _offer(201 * U, FeeMath.Payer.Seller);
        _expectCreateRevert(o, Escrow.BadOffer.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.confirmWindow = 1 days - 1;
        _expectCreateRevert(o, Escrow.BadOffer.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.confirmWindow = 30 days + 1;
        _expectCreateRevert(o, Escrow.BadOffer.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.shipWindow = 1 days - 1;
        _expectCreateRevert(o, Escrow.BadOffer.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.shipWindow = 30 days + 1;
        _expectCreateRevert(o, Escrow.BadOffer.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.maxFeeBps = 99;
        _expectCreateRevert(o, Escrow.FeeLimit.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.maxBaseGas = 99_999;
        _expectCreateRevert(o, Escrow.FeeLimit.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.maxMinFee = 1 * U - 1;
        _expectCreateRevert(o, Escrow.FeeLimit.selector);

        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.maxFills = 0;
        _expectCreateRevert(o, MarketBase.OfferUnavailable.selector);
    }

    /// Plan §6: an offer signed before a (queued) minimum-fee raise must not be charged the higher fee.
    function test_maxMinFee_protectsOldOffer() public {
        Escrow.Offer memory old = _offer(50 * U, FeeMath.Payer.Seller);
        old.expiry = uint64(block.timestamp + 30 days); // public offers live up to 30 days
        bytes memory oldSig = _sign(old, sellerKey);

        EscrowConfig.Params memory p = defaultParams();
        p.minFee = uint64(2 * U);
        vm.prank(owner);
        config.setParams(p);
        vm.warp(block.timestamp + 72 hours);
        config.applyParams();

        vm.prank(buyer);
        vm.expectRevert(Escrow.FeeLimit.selector);
        escrow.createAndFund(old, oldSig);

        Escrow.Offer memory fresh = _offer(50 * U, FeeMath.Payer.Seller);
        fresh.maxMinFee = 2 * U;
        fresh.nonce = 2;
        fresh.expiry = uint64(block.timestamp + 1 days);
        uint256 id = _create(fresh);
        assertEq(escrow.getOrder(id).sellerFee, 2 * U);
    }

    function test_maxMinFee_ignoredDuringCampaign() public {
        _startCampaign(100, 0);
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.maxMinFee = 0;
        _create(o);
    }

    function test_revert_create_cancelledNonce() public {
        vm.prank(seller);
        escrow.cancelOffer(1);
        assertTrue(escrow.cancelledNonce(seller, 1));
        _expectCreateRevert(_offer(100 * U, FeeMath.Payer.Seller), MarketBase.OfferUnavailable.selector);
    }

    function test_revert_create_badSignature() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        (, uint256 otherKey) = makeAddrAndKey("other");
        bytes memory sig = _sign(o, otherKey);
        vm.prank(buyer);
        vm.expectRevert(MarketBase.BadSignature.selector);
        escrow.createAndFund(o, sig);
    }

    function test_revert_create_signatureBoundToChain() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        bytes memory sig = _sign(o, sellerKey);
        vm.chainId(421_614); // Arbitrum Sepolia: same offer must not replay
        vm.prank(buyer);
        vm.expectRevert(MarketBase.BadSignature.selector);
        escrow.createAndFund(o, sig);
    }

    function test_revert_create_paused() public {
        vm.prank(owner);
        config.setPaused(true);
        _expectCreateRevert(_offer(100 * U, FeeMath.Payer.Seller), EscrowConfig.IsPaused.selector);
    }

    function test_revert_create_feesExceedAmount() public {
        EscrowConfig.Params memory p = defaultParams();
        p.minOrder = uint64(1 * U);
        vm.prank(owner);
        config.setParams(p);
        // 1.05 USDT order: seller-borne min fee 1 + gas 0.1 >= amount
        _expectCreateRevert(_offer(1_050_000, FeeMath.Payer.Seller), Escrow.FeesExceedAmount.selector);
    }

    function test_revert_create_amountMismatch() public {
        FeeOnTransferToken fot = new FeeOnTransferToken();
        (, Escrow e) = _deploy(address(fot), defaultParams());
        fot.mint(buyer, 1000 * U);
        vm.prank(buyer);
        fot.approve(address(e), type(uint256).max);
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.token = address(fot);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(sellerKey, e.hashOffer(o));
        vm.prank(buyer);
        vm.expectRevert(MarketBase.AmountMismatch.selector);
        e.createAndFund(o, abi.encodePacked(r, s, v));
    }
}

contract EscrowFlowTest is EscrowBase {
    uint256 internal id;

    function setUp() public override {
        super.setUp();
        id = _createDefault();
    }

    function test_markDelivered() public {
        _deliver(id);
        Escrow.Order memory o = escrow.getOrder(id);
        assertEq(o.confirmDeadline, block.timestamp + 7 days);
        assertTrue(o.platformPaid);
    }

    function test_revert_markDelivered() public {
        vm.prank(buyer);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.markDelivered(id);

        vm.warp(block.timestamp + 3 days + 1);
        vm.prank(seller);
        vm.expectRevert(Escrow.TooLate.selector);
        escrow.markDelivered(id);
    }

    function test_revert_markDelivered_twice() public {
        _deliver(id);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.markDelivered(id);
    }

    function test_release_afterDelivery() public {
        _deliver(id);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        escrow.release(id);
        assertEq(_bal(seller) - s0, 98_900_000);
        assertEq(_bal(feeRecipient), 1_100_000);
        assertEq(_bal(address(escrow)), 0);
        assertEq(uint8(escrow.getOrder(id).state), uint8(Escrow.State.Released));
    }

    function test_release_beforeDelivery() public {
        vm.prank(buyer);
        escrow.release(id);
        assertEq(_bal(feeRecipient), 1_100_000);
    }

    function test_revert_release() public {
        vm.prank(seller);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.release(id);
        vm.prank(buyer);
        escrow.release(id);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.release(id);
    }

    function test_release_halfPayer() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Half);
        o.nonce = 9;
        uint256 id2 = _create(o);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        escrow.release(id2);
        assertEq(_bal(seller) - s0, 99_450_000);
        assertEq(_bal(feeRecipient), 1_100_000);
    }

    function test_refund_undelivered_noFeeNoGas() public {
        uint256 b0 = _bal(buyer);
        vm.prank(seller);
        escrow.refund(id);
        assertEq(_bal(buyer) - b0, 100 * U);
        assertEq(_bal(feeRecipient), 0);
        assertEq(uint8(escrow.getOrder(id).state), uint8(Escrow.State.Refunded));
    }

    function test_refund_delivered_gasRecovered() public {
        _deliver(id);
        uint256 b0 = _bal(buyer);
        vm.prank(seller);
        escrow.refund(id);
        assertEq(_bal(buyer) - b0, 99_900_000);
        assertEq(_bal(feeRecipient), 100_000);
    }

    function test_refund_buyerPays_refundsPrepaidFee() public {
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Buyer);
        o.nonce = 9;
        uint256 b0 = _bal(buyer);
        uint256 id2 = _create(o);
        vm.prank(seller);
        escrow.refund(id2);
        assertEq(_bal(buyer), b0);
    }

    function test_revert_refund_notSeller() public {
        vm.prank(buyer);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.refund(id);
    }

    function test_claimTimeout_release() public {
        _deliver(id);
        vm.warp(block.timestamp + 7 days);
        vm.expectRevert(Escrow.TooEarly.selector);
        escrow.claimTimeout(id);
        vm.warp(block.timestamp + 1);
        vm.prank(stranger);
        escrow.claimTimeout(id);
        assertEq(uint8(escrow.getOrder(id).state), uint8(Escrow.State.Released));
        assertEq(_bal(feeRecipient), 1_100_000);
    }

    function test_claimTimeout_refundUndelivered() public {
        uint256 b0 = _bal(buyer);
        vm.warp(block.timestamp + 3 days);
        vm.expectRevert(Escrow.TooEarly.selector);
        escrow.claimTimeout(id);
        vm.warp(block.timestamp + 1);
        escrow.claimTimeout(id);
        assertEq(_bal(buyer) - b0, 100 * U);
        assertEq(uint8(escrow.getOrder(id).state), uint8(Escrow.State.Refunded));
    }

    function test_revert_claimTimeout_disputed() public {
        vm.prank(buyer);
        escrow.dispute(id);
        vm.warp(block.timestamp + 365 days);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.claimTimeout(id);
    }

    function test_extendConfirm_seller() public {
        _deliver(id);
        uint64 d0 = escrow.getOrder(id).confirmDeadline;
        vm.prank(seller);
        escrow.extendConfirm(id, 10 days);
        vm.prank(seller);
        escrow.extendConfirm(id, 20 days);
        assertEq(escrow.getOrder(id).confirmDeadline, d0 + 30 days);
        assertEq(escrow.getOrder(id).sellerExtended, 30 days);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.extendConfirm(id, 1);
    }

    function test_revert_extendConfirm_seller() public {
        vm.prank(seller);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.extendConfirm(id, 1 days);
        _deliver(id);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.extendConfirm(id, 0);
        vm.prank(stranger);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.extendConfirm(id, 1 days);
        vm.warp(block.timestamp + 7 days + 1);
        vm.prank(seller);
        vm.expectRevert(Escrow.TooLate.selector);
        escrow.extendConfirm(id, 1 days);
    }

    function test_extendConfirm_buyerEmergencyOnce() public {
        _deliver(id);
        uint64 d0 = escrow.getOrder(id).confirmDeadline;
        vm.warp(d0 - 6 hours - 1);
        vm.prank(buyer);
        vm.expectRevert(Escrow.TooEarly.selector);
        escrow.extendConfirm(id, 0);
        vm.warp(d0 - 6 hours);
        vm.prank(buyer);
        escrow.extendConfirm(id, 0);
        assertEq(escrow.getOrder(id).confirmDeadline, d0 + 24 hours);
        vm.warp(d0 + 20 hours);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.extendConfirm(id, 0);
    }

    function test_proposeAccept_partialRefund() public {
        vm.prank(buyer);
        escrow.proposeEnd(id, 40 * U);
        uint256 b0 = _bal(buyer);
        uint256 s0 = _bal(seller);
        vm.prank(seller);
        escrow.acceptEnd(id, 40 * U);
        assertEq(_bal(buyer) - b0, 40 * U);
        assertEq(_bal(seller) - s0, 59_300_000);
        assertEq(_bal(feeRecipient), 700_000);
        assertEq(uint8(escrow.getOrder(id).state), uint8(Escrow.State.Settled));
    }

    function test_proposeEnd_overwrite() public {
        vm.prank(buyer);
        escrow.proposeEnd(id, 40 * U);
        vm.prank(seller);
        escrow.proposeEnd(id, 10 * U);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadState.selector); // proposer cannot accept own proposal
        escrow.acceptEnd(id, 10 * U);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadParams.selector); // stale amount
        escrow.acceptEnd(id, 40 * U);
        vm.prank(buyer);
        escrow.acceptEnd(id, 10 * U);
    }

    function test_revert_proposeAccept() public {
        vm.prank(stranger);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.proposeEnd(id, 1);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.proposeEnd(id, 100 * U + 1);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadState.selector); // nothing proposed
        escrow.acceptEnd(id, 0);
        vm.prank(buyer);
        escrow.proposeEnd(id, 50 * U);
        vm.prank(stranger);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.acceptEnd(id, 50 * U);
        vm.warp(block.timestamp + 72 hours + 1);
        vm.prank(seller);
        vm.expectRevert(Escrow.TooLate.selector);
        escrow.acceptEnd(id, 50 * U);
    }

    function test_acceptEnd_duringDispute_refundsDisputeFee() public {
        vm.prank(buyer);
        escrow.dispute(id);
        vm.prank(seller);
        escrow.proposeEnd(id, 100 * U);
        uint256 b0 = _bal(buyer);
        vm.prank(buyer);
        escrow.acceptEnd(id, 100 * U);
        // principal + dispute fee back; gas (sponsored proposal) recovered from the pool
        assertEq(_bal(buyer) - b0, 100 * U + 1 * U - 100_000);
        assertEq(_bal(feeRecipient), 100_000);
    }

    function test_dispute_buyerPaysFee() public {
        uint256 b0 = _bal(buyer);
        vm.prank(buyer);
        escrow.dispute(id);
        assertEq(b0 - _bal(buyer), 1 * U);
        Escrow.Order memory o = escrow.getOrder(id);
        assertEq(uint8(o.state), uint8(Escrow.State.Disputed));
        assertEq(o.disputedAt, block.timestamp);
        assertFalse(o.sellerDisputed);
    }

    function test_revert_dispute() public {
        vm.prank(stranger);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.dispute(id);
        vm.warp(block.timestamp + 3 days + 1);
        vm.prank(buyer);
        vm.expectRevert(Escrow.TooLate.selector);
        escrow.dispute(id);
    }

    function test_revert_dispute_afterConfirmWindow_orTwice() public {
        _deliver(id);
        vm.prank(seller);
        escrow.dispute(id);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.dispute(id);
    }

    function test_resolve_buyerDisputerWins() public {
        _deliver(id);
        uint256 b0 = _bal(buyer);
        vm.prank(buyer);
        escrow.dispute(id);
        vm.prank(arbiter);
        escrow.resolve(id, 10_000, 0);
        assertEq(_bal(buyer), b0 + 100 * U - 100_000);
        assertEq(_bal(feeRecipient), 100_000);
    }

    function test_resolve_buyerDisputerLoses_onlyOneCharge() public {
        _deliver(id);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        escrow.dispute(id);
        vm.prank(arbiter);
        escrow.resolve(id, 0, 0);
        assertEq(_bal(seller) - s0, 99_900_000); // fee waived: dispute fee retained instead
        assertEq(_bal(feeRecipient), 1_100_000);
    }

    function test_resolve_sellerDisputer_split() public {
        _deliver(id);
        uint256 s0 = _bal(seller);
        uint256 b0 = _bal(buyer);
        vm.prank(seller);
        escrow.dispute(id);
        vm.prank(arbiter);
        escrow.resolve(id, 5000, 0);
        assertEq(_bal(buyer) - b0, 50 * U);
        assertEq(int256(_bal(seller)) - int256(s0), 50_400_000 - 1_000_000);
        assertEq(_bal(feeRecipient), 600_000);
    }

    function test_revert_resolve() public {
        vm.prank(arbiter);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.resolve(id, 0, 0);
        vm.prank(buyer);
        escrow.dispute(id);
        vm.prank(owner);
        vm.expectRevert(Escrow.NotParty.selector);
        escrow.resolve(id, 0, 0);
        vm.prank(arbiter);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.resolve(id, 10_001, 0);
    }

    function test_resolveTimeout_undelivered_fullRefund() public {
        uint256 b0 = _bal(buyer);
        vm.prank(buyer);
        escrow.dispute(id);
        vm.warp(block.timestamp + 30 days);
        vm.expectRevert(Escrow.TooEarly.selector);
        escrow.resolveTimeout(id);
        vm.warp(block.timestamp + 1);
        escrow.resolveTimeout(id);
        assertEq(_bal(buyer), b0 + 100 * U);
        assertEq(_bal(feeRecipient), 0);
    }

    function test_resolveTimeout_delivered_half() public {
        _deliver(id);
        uint256 b0 = _bal(buyer);
        uint256 s0 = _bal(seller);
        vm.prank(buyer);
        escrow.dispute(id);
        vm.warp(block.timestamp + 30 days + 1);
        escrow.resolveTimeout(id);
        assertEq(_bal(buyer), b0 + 50 * U);
        assertEq(_bal(seller) - s0, 49_400_000);
        assertEq(_bal(feeRecipient), 600_000);
    }

    function test_revert_resolveTimeout_notDisputed() public {
        vm.expectRevert(Escrow.BadState.selector);
        escrow.resolveTimeout(id);
    }

    function test_frozenRecipient_creditAndWithdraw() public {
        usdt.setFrozen(seller, true);
        vm.prank(buyer);
        escrow.release(id);
        assertEq(escrow.credit(seller, address(usdt)), 98_900_000);
        assertEq(_bal(feeRecipient), 1_100_000); // other recipients unaffected
        address fresh = makeAddr("fresh");
        vm.prank(seller);
        escrow.withdraw(address(usdt), fresh);
        assertEq(_bal(fresh), 98_900_000);
        assertEq(escrow.credit(seller, address(usdt)), 0);
        vm.prank(seller);
        vm.expectRevert(MarketBase.NothingToWithdraw.selector);
        escrow.withdraw(address(usdt), fresh);
    }

    function test_pause_keepsExistingOrdersWorking() public {
        vm.prank(owner);
        config.setPaused(true);
        assertTrue(config.paused());
        _deliver(id);
        vm.prank(buyer);
        escrow.release(id);
        vm.prank(owner);
        config.setPaused(false);
        assertFalse(config.paused());
    }

    function test_revert_unknownOrder() public {
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadState.selector);
        escrow.release(999);
    }
}

contract EscrowParamsTest is EscrowBase {
    function test_constructor_rejects() public {
        vm.expectRevert(Escrow.BadParams.selector);
        new Escrow(EscrowConfig(address(0)), arbiter, feeRecipient);
        vm.expectRevert(Escrow.BadParams.selector);
        new Escrow(config, address(0), feeRecipient);
        vm.expectRevert(Escrow.BadParams.selector);
        new Escrow(config, arbiter, address(0));
        address[] memory none;
        EscrowConfig.Params memory p = defaultParams();
        p.feeBps = 1001;
        vm.expectRevert(EscrowConfig.BadParams.selector);
        new EscrowConfig(owner, p, address(usdt), platform, none);
        vm.expectRevert(EscrowConfig.BadParams.selector);
        new EscrowConfig(owner, defaultParams(), address(0), platform, none);
    }

    function test_hardCaps() public {
        EscrowConfig.Params[16] memory bad;
        for (uint256 i; i < 16; i++) {
            bad[i] = defaultParams();
        }
        bad[0].feeBps = 1001;
        bad[1].totalLockedCap = uint64(200 * U - 1); // below the single-order cap
        bad[2].minFee = uint64(5 * U + 1);
        bad[3].minOrder = uint64(1 * U - 1);
        bad[4].minOrder = uint64(100 * U + 1);
        bad[5].orderCap = uint64(4 * U);
        bad[6].orderCap = uint64(100_000 * U + 1);
        bad[7].baseGas = uint64(1 * U + 1);
        bad[8].disputeMulBps = 4999;
        bad[9].disputeMulBps = 50_001;
        bad[10].resolveTimeout = 14 days - 1;
        bad[11].resolveTimeout = 60 days + 1;
        bad[12].proposalWindow = 1 days - 1;
        bad[13].proposalWindow = 7 days + 1;
        bad[14].totalLockedCap = uint64(1_000_000 * U + 1);
        bad[15].claimGas = uint64(1 * U + 1);
        for (uint256 i; i < 16; i++) {
            vm.prank(owner);
            vm.expectRevert(EscrowConfig.BadParams.selector);
            config.setParams(bad[i]);
        }
    }

    function test_tightening_isQueued72h() public {
        EscrowConfig.Params memory p = defaultParams();
        p.feeBps = 200;
        vm.prank(owner);
        config.setParams(p);
        assertEq(config.params().feeBps, 100);
        assertEq(config.pendingParams().feeBps, 200);
        assertEq(config.pendingEta(), block.timestamp + 72 hours);
        vm.warp(block.timestamp + 72 hours - 1);
        vm.expectRevert(EscrowConfig.TooEarly.selector);
        config.applyParams();
        vm.warp(block.timestamp + 1);
        vm.prank(stranger);
        config.applyParams();
        assertEq(config.params().feeBps, 200);
        assertEq(config.pendingEta(), 0);
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.applyParams();
    }

    function test_cancelParams() public {
        EscrowConfig.Params memory p = defaultParams();
        p.baseGas = 200_000;
        vm.prank(owner);
        config.setParams(p);
        vm.prank(owner);
        config.cancelParams();
        assertEq(config.pendingEta(), 0);
        vm.prank(owner);
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.cancelParams();
    }

    function test_eachTighteningField_isQueued() public {
        EscrowConfig.Params[8] memory t;
        for (uint256 i; i < 8; i++) {
            t[i] = defaultParams();
        }
        t[0].minFee = uint64(2 * U);
        t[1].minOrder = uint64(6 * U);
        t[2].baseGas = 100_001;
        t[3].disputeMulBps = 10_001;
        t[4].resolveTimeout = 29 days;
        t[5].proposalWindow = 71 hours;
        t[6].feeBps = 101;
        t[7].claimGas = 20_001;
        for (uint256 i; i < 8; i++) {
            vm.prank(owner);
            config.setParams(t[i]);
            assertGt(config.pendingEta(), 0);
            assertEq(config.params().minFee, 1 * U);
            vm.prank(owner);
            config.cancelParams();
        }
    }

    /// Lead decision: both caps are risk brakes; raising and lowering apply at once (only new orders are affected).
    function test_caps_changeImmediately() public {
        EscrowConfig.Params memory p = defaultParams();
        p.orderCap = uint64(100 * U);
        p.totalLockedCap = uint64(1000 * U);
        vm.prank(owner);
        config.setParams(p);
        assertEq(config.params().orderCap, 100 * U);
        assertEq(config.params().totalLockedCap, 1000 * U);
        assertEq(config.pendingEta(), 0);
        p.orderCap = uint64(500 * U);
        p.totalLockedCap = uint64(50_000 * U);
        vm.prank(owner);
        config.setParams(p);
        assertEq(config.params().totalLockedCap, 50_000 * U);
        assertEq(config.pendingEta(), 0);
    }

    function test_onlyOwner() public {
        EscrowConfig.Params memory p = defaultParams();
        vm.startPrank(stranger);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        config.setParams(p);
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        config.cancelParams();
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger));
        config.setPaused(true);
        vm.stopPrank();
    }
}
