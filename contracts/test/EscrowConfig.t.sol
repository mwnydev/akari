// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FreezableUSDT, Token18} from "./mocks/TestTokens.sol";
import {EscrowBase} from "./EscrowBase.sol";

/// Campaign scheduling, token whitelist and the generic 72h queue (plan §6「手续费限时免费活动」「代币白名单」).
contract EscrowConfigTest is EscrowBase {
    uint8 internal constant K_CAMPAIGN = 1;
    uint8 internal constant K_TOKEN = 2;
    uint8 internal constant K_SIGNER = 3;

    function _c(uint256 start, uint256 end, uint16 rate, uint8 scope, uint32 subsidy)
        internal
        pure
        returns (EscrowConfig.Campaign memory)
    {
        return EscrowConfig.Campaign({
            start: uint64(start), end: uint64(end), rateBps: rate, scope: scope, gasSubsidyCap: subsidy
        });
    }

    function _setCampaign(EscrowConfig.Campaign memory c) internal {
        vm.prank(owner);
        config.setCampaign(c);
    }

    function _nextOffer(uint256 nonce) internal view returns (Escrow.Offer memory o) {
        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.nonce = nonce;
    }

    // ───────────── campaign ─────────────

    function test_campaign_scopeAndRate() public {
        _setCampaign(_c(block.timestamp, block.timestamp + 10 days, 0, 2, 0)); // auctions only
        uint256 id = _create(_nextOffer(1));
        assertEq(escrow.getOrder(id).sellerFee, 1 * U);

        _setCampaign(_c(block.timestamp, block.timestamp + 10 days, 0, 3, 0)); // widen: immediate
        id = _create(_nextOffer(2));
        assertEq(escrow.getOrder(id).sellerFee, 0);
    }

    function test_campaign_rateAboveBaseUsesBase_minFeeOff() public {
        _setCampaign(_c(block.timestamp, block.timestamp + 10 days, 500, 1, 0));
        EscrowConfig.Quote memory q = config.preview(1);
        assertEq(q.feeBps, 100);
        assertEq(q.minFee, 0);
        assertTrue(q.campaign);
        Escrow.Offer memory o = _offer(50 * U, FeeMath.Payer.Seller);
        uint256 id = _create(o);
        assertEq(escrow.getOrder(id).sellerFee, 500_000); // 1% of 50, no 1 USDT minimum
    }

    function test_campaign_futureAndExpired() public {
        _setCampaign(_c(block.timestamp + 1 days, block.timestamp + 2 days, 0, 3, 0));
        assertFalse(config.preview(1).campaign);
        vm.warp(block.timestamp + 1 days);
        assertTrue(config.preview(1).campaign);
        vm.warp(block.timestamp + 1 days);
        assertFalse(config.preview(1).campaign);
    }

    function test_campaign_gasSubsidyCountsOrders() public {
        _setCampaign(_c(block.timestamp, block.timestamp + 10 days, 100, 3, 1));
        assertEq(config.preview(1).baseGas, 0);
        assertEq(config.preview(1).claimGas, 0);
        uint256 id = _create(_nextOffer(1));
        assertEq(escrow.getOrder(id).sellerGas, 0);
        assertEq(config.subsidyUsed(), 1);
        assertEq(config.preview(1).baseGas, 100_000);
        assertEq(config.preview(1).claimGas, 20_000);
        id = _create(_nextOffer(2));
        assertEq(escrow.getOrder(id).sellerGas, 100_000);
        // a subsidized order: platform still sponsors, nothing is charged for gas
        _deliver(1);
        vm.prank(buyer);
        escrow.release(1);
        assertEq(_bal(feeRecipient), 1 * U);
    }

    function test_campaign_endEarlyIsQueued() public {
        EscrowConfig.Campaign memory cur = _c(block.timestamp, block.timestamp + 10 days, 0, 3, 5);
        _setCampaign(cur);
        _create(_nextOffer(1));
        EscrowConfig.Campaign memory none;
        _setCampaign(none);
        assertEq(config.campaign().end, cur.end); // still running
        bytes32 key = config.queueKey(K_CAMPAIGN, abi.encode(cur, none));
        assertEq(config.queuedEta(key), block.timestamp + 72 hours);

        vm.expectRevert(EscrowConfig.TooEarly.selector);
        config.applyCampaign(none);
        vm.warp(block.timestamp + 72 hours);
        vm.prank(stranger);
        config.applyCampaign(none);
        assertEq(config.campaign().end, 0);
        assertEq(config.subsidyUsed(), 0);
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.applyCampaign(none);
    }

    function test_campaign_lessGenerousVariantsQueued() public {
        EscrowConfig.Campaign memory cur = _c(block.timestamp, block.timestamp + 10 days, 50, 1, 5);
        _setCampaign(cur);
        EscrowConfig.Campaign[5] memory worse = [
            _c(block.timestamp + 1, block.timestamp + 10 days, 50, 1, 5), // starts later
            _c(block.timestamp, block.timestamp + 9 days, 50, 1, 5), // ends earlier
            _c(block.timestamp, block.timestamp + 10 days, 51, 1, 5), // higher rate
            _c(block.timestamp, block.timestamp + 10 days, 50, 2, 5), // drops a scope
            _c(block.timestamp, block.timestamp + 10 days, 50, 1, 4) // fewer subsidies
        ];
        for (uint256 i; i < 5; i++) {
            _setCampaign(worse[i]);
            assertEq(config.campaign().rateBps, 50);
            assertEq(config.campaign().end, cur.end);
            assertEq(config.campaign().scope, 1);
        }
    }

    function test_campaign_generousExtensionKeepsSubsidyCount() public {
        _setCampaign(_c(block.timestamp, block.timestamp + 10 days, 50, 1, 5));
        _create(_nextOffer(1));
        assertEq(config.subsidyUsed(), 1);
        _setCampaign(_c(block.timestamp, block.timestamp + 20 days, 0, 3, 10));
        assertEq(config.campaign().end, block.timestamp + 20 days);
        assertEq(config.subsidyUsed(), 1);
    }

    function test_campaign_staleQueuedChangeVoid() public {
        EscrowConfig.Campaign memory cur = _c(block.timestamp, block.timestamp + 10 days, 50, 1, 0);
        _setCampaign(cur);
        EscrowConfig.Campaign memory worse = _c(block.timestamp, block.timestamp + 5 days, 50, 1, 0);
        _setCampaign(worse); // queued against `cur`
        _setCampaign(_c(block.timestamp, block.timestamp + 30 days, 50, 1, 0)); // generous: immediate
        vm.warp(block.timestamp + 72 hours);
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.applyCampaign(worse);
    }

    function test_revert_campaign_bounds() public {
        uint256 t = block.timestamp;
        EscrowConfig.Campaign[9] memory bad = [
            _c(t + 1, t + 1, 0, 1, 0), // end <= start
            _c(t, t + 90 days + 1, 0, 1, 0), // > 90 days
            _c(0, t, 0, 1, 0), // already over
            _c(t, t + 1 days, 1001, 1, 0), // rate above hard cap
            _c(t, t + 1 days, 0, 0, 0), // no scope
            _c(t, t + 1 days, 0, 4, 0), // unknown scope
            _c(t, t + 1 days, 0, 1, 10_001), // subsidy above cap
            _c(1, 0, 0, 0, 0), // "none" must be all zero
            _c(0, 0, 1, 0, 0)
        ];
        for (uint256 i; i < 9; i++) {
            vm.prank(owner);
            vm.expectRevert(EscrowConfig.BadParams.selector);
            config.setCampaign(bad[i]);
        }
        EscrowConfig.Campaign[2] memory moreNone = [_c(0, 0, 0, 1, 0), _c(0, 0, 0, 0, 1)];
        for (uint256 i; i < 2; i++) {
            vm.prank(owner);
            vm.expectRevert(EscrowConfig.BadParams.selector);
            config.setCampaign(moreNone[i]);
        }
        _setCampaign(_c(0, 0, 0, 0, 0)); // clearing with no campaign running: fine
    }

    // ───────────── tokens ─────────────

    function test_token_whitelistQueued() public {
        FreezableUSDT usdc = new FreezableUSDT();
        vm.prank(owner);
        config.queueToken(address(usdc));
        vm.expectRevert(EscrowConfig.TooEarly.selector);
        config.enableToken(address(usdc));
        vm.warp(block.timestamp + 72 hours);
        config.enableToken(address(usdc));
        assertTrue(config.tokenAllowed(address(usdc)));

        usdc.mint(buyer, 1000 * U);
        vm.prank(buyer);
        usdc.approve(address(escrow), type(uint256).max);
        Escrow.Offer memory o = _offer(100 * U, FeeMath.Payer.Seller);
        o.token = address(usdc);
        o.expiry = uint64(block.timestamp + 1 days);
        uint256 id = _create(o);
        assertEq(escrow.getOrder(id).token, address(usdc));
        usdc.setFrozen(seller, true);
        vm.prank(buyer);
        escrow.release(id);
        assertEq(escrow.credit(seller, address(usdc)), 98_900_000);
        assertEq(escrow.credit(seller, address(usdt)), 0);
        assertEq(usdc.balanceOf(feeRecipient), 1_100_000);
    }

    function test_token_disableImmediate_existingOrdersSettle() public {
        uint256 id = _createDefault();
        vm.prank(owner);
        config.disableToken(address(usdt));
        Escrow.Offer memory o = _nextOffer(2);
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        vm.expectRevert(EscrowConfig.TokenNotAllowed.selector);
        escrow.createAndFund(o, sig);
        vm.prank(seller);
        vm.expectRevert(Escrow.BadParams.selector);
        escrow.depositBond(address(usdt), 1);
        vm.prank(buyer);
        escrow.release(id);
    }

    function test_revert_token() public {
        Token18 t18 = new Token18();
        vm.startPrank(owner);
        vm.expectRevert(EscrowConfig.BadParams.selector);
        config.queueToken(address(t18));
        vm.expectRevert(EscrowConfig.BadParams.selector);
        config.queueToken(address(0));
        vm.stopPrank();
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.enableToken(address(t18));
    }

    // ───────────── auction signer, cancel, ownership ─────────────

    function test_auctionSigner_queued_andStaleVoid() public {
        address a = makeAddr("signerA");
        address b = makeAddr("signerB");
        vm.startPrank(owner);
        config.queueAuctionSigner(a);
        config.queueAuctionSigner(b);
        vm.stopPrank();
        vm.warp(block.timestamp + 72 hours);
        config.applyAuctionSigner(b);
        assertEq(config.auctionSigner(), b);
        vm.expectRevert(EscrowConfig.BadState.selector); // keyed on the old signer
        config.applyAuctionSigner(a);
    }

    function test_cancelQueued() public {
        vm.prank(owner);
        config.queueAuctionSigner(stranger);
        bytes32 key = config.queueKey(K_SIGNER, abi.encode(platform, stranger));
        vm.prank(owner);
        config.cancelQueued(key);
        vm.warp(block.timestamp + 72 hours);
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.applyAuctionSigner(stranger);
        vm.prank(owner);
        vm.expectRevert(EscrowConfig.BadState.selector);
        config.cancelQueued(key);
    }

    function test_onlyOwner() public {
        EscrowConfig.Params memory p = defaultParams();
        EscrowConfig.Campaign memory c;
        bytes memory err = abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, stranger);
        vm.startPrank(stranger);
        vm.expectRevert(err);
        config.setParams(p);
        vm.expectRevert(err);
        config.cancelParams();
        vm.expectRevert(err);
        config.setPaused(true);
        vm.expectRevert(err);
        config.setCampaign(c);
        vm.expectRevert(err);
        config.queueToken(address(usdt));
        vm.expectRevert(err);
        config.disableToken(address(usdt));
        vm.expectRevert(err);
        config.queueAuctionSigner(stranger);
        vm.expectRevert(err);
        config.queueMarket(stranger);
        vm.expectRevert(err);
        config.cancelQueued(bytes32(0));
        vm.stopPrank();
    }

    function test_tokenKey() public view {
        assertEq(
            config.queueKey(K_TOKEN, abi.encode(address(usdt))),
            keccak256(abi.encode(K_TOKEN, abi.encode(address(usdt))))
        );
    }
}
