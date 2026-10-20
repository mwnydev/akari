// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {MarketBase} from "../src/MarketBase.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {EscrowBase} from "./EscrowBase.sol";
import {stdJson} from "forge-std/StdJson.sol";

/// Auction settlement (plan §6「拍卖成交」): bids stay off-chain; the winner pays with seller auth + platform proof.
contract EscrowAuctionTest is EscrowBase {
    using stdJson for string;

    uint32 internal constant RATE = 72_000; // 7.2000 CNY per USDT

    address internal winner;

    function setUp() public override {
        super.setUp();
        winner = makeAddr("winner");
        _fund(winner, 1_000_000 * U);
    }

    function _auth() internal view returns (Escrow.AuctionAuth memory) {
        return Escrow.AuctionAuth({
            seller: seller,
            token: address(usdt),
            auctionId: 42,
            reservePrice: 50 * U,
            currency: 0,
            cnyPerUsdt: 0,
            feePayer: FeeMath.Payer.Seller,
            maxFeeBps: 100,
            maxBaseGas: 100_000,
            maxMinFee: 1 * U,
            instant: false,
            shipWindow: 3 days,
            confirmWindow: 7 days,
            listingHash: keccak256("auction listing"),
            expiry: uint64(block.timestamp + 10 days)
        });
    }

    /// CNY auction (ADR 0011): reserve 360 CNY (= 50 USDT at 7.2), prices in fen.
    function _cnyAuth(uint32 rate) internal view returns (Escrow.AuctionAuth memory a) {
        a = _auth();
        a.currency = escrow.CURRENCY_CNY();
        a.cnyPerUsdt = rate;
        a.reservePrice = 36_000;
    }

    function _proof(Escrow.AuctionAuth memory a, address who, uint256 price)
        internal
        view
        returns (Escrow.SettlementProof memory)
    {
        return Escrow.SettlementProof({
            authHash: escrow.hashAuction(a), winner: who, price: price, payDeadline: uint64(block.timestamp + 48 hours)
        });
    }

    function _sig(bytes32 digest, uint256 key) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function _settle(Escrow.AuctionAuth memory a, Escrow.SettlementProof memory p, address caller)
        internal
        returns (uint256)
    {
        bytes memory sellerSig = _sig(escrow.hashAuction(a), sellerKey);
        bytes memory platformSig = _sig(escrow.hashProof(p), platformKey);
        vm.prank(caller);
        return escrow.createFromAuction(a, sellerSig, p, platformSig);
    }

    function _expectSettleRevert(Escrow.AuctionAuth memory a, Escrow.SettlementProof memory p, bytes4 err) internal {
        bytes memory sellerSig = _sig(escrow.hashAuction(a), sellerKey);
        bytes memory platformSig = _sig(escrow.hashProof(p), platformKey);
        vm.prank(winner);
        vm.expectRevert(err);
        escrow.createFromAuction(a, sellerSig, p, platformSig);
    }

    function test_settle_opensOneTimeOrder() public {
        Escrow.AuctionAuth memory a = _auth();
        uint256 w0 = _bal(winner);
        uint256 id = _settle(a, _proof(a, winner, 80 * U), winner);
        Escrow.Order memory o = escrow.getOrder(id);
        assertEq(o.buyer, winner);
        assertEq(o.seller, seller);
        assertEq(o.amount, 80 * U);
        assertEq(o.sellerFee, 1 * U);
        assertEq(w0 - _bal(winner), 80 * U);
        assertTrue(escrow.auctionSettled(keccak256(abi.encode(seller, uint256(42)))));
        // from here it is an ordinary order
        _deliver(id);
        vm.prank(winner);
        escrow.release(id);
    }

    function test_settle_onlyOnce_evenForRunnerUp() public {
        Escrow.AuctionAuth memory a = _auth();
        _settle(a, _proof(a, winner, 80 * U), winner);
        _fund(stranger, 1000 * U);
        Escrow.SettlementProof memory p2 = _proof(a, stranger, 70 * U);
        bytes memory sellerSig = _sig(escrow.hashAuction(a), sellerKey);
        bytes memory platformSig = _sig(escrow.hashProof(p2), platformKey);
        vm.prank(stranger);
        vm.expectRevert(MarketBase.OfferUnavailable.selector);
        escrow.createFromAuction(a, sellerSig, p2, platformSig);
    }

    function test_revert_settle_checks() public {
        Escrow.AuctionAuth memory a = _auth();
        Escrow.SettlementProof memory p;

        p = _proof(a, winner, 80 * U);
        p.authHash = keccak256("another auction");
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);
        p.authHash = bytes32(0); // equality, not order: hashes and winners below and above the real ones
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);
        p.authHash = bytes32(type(uint256).max);
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);

        p = _proof(a, stranger, 80 * U); // caller is not the winner
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);
        p = _proof(a, address(1), 80 * U);
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);
        p = _proof(a, address(type(uint160).max), 80 * U);
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);

        p = _proof(a, winner, 50 * U - 1); // below reserve
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);

        p = _proof(a, winner, 80 * U);
        p.payDeadline = uint64(block.timestamp - 1);
        _expectSettleRevert(a, p, MarketBase.OfferUnavailable.selector);

        Escrow.AuctionAuth memory expired = _auth();
        expired.expiry = uint64(block.timestamp - 1);
        _expectSettleRevert(expired, _proof(expired, winner, 80 * U), MarketBase.OfferUnavailable.selector);
        p = _proof(expired, winner, 80 * U);
        p.payDeadline = uint64(block.timestamp - 1); // both expired at once
        _expectSettleRevert(expired, p, MarketBase.OfferUnavailable.selector);

        Escrow.AuctionAuth memory sellerBuys = _auth();
        sellerBuys.seller = winner; // signature check fails first: the winner is not `seller`'s key
        _expectSettleRevert(sellerBuys, _proof(sellerBuys, winner, 80 * U), MarketBase.BadSignature.selector);

        Escrow.AuctionAuth memory tooBig = _auth();
        tooBig.auctionId = 43;
        _expectSettleRevert(tooBig, _proof(tooBig, winner, 201 * U), Escrow.BadOffer.selector); // over order cap
    }

    function test_revert_settle_badSignatures() public {
        Escrow.AuctionAuth memory a = _auth();
        Escrow.SettlementProof memory p = _proof(a, winner, 80 * U);
        (, uint256 otherKey) = makeAddrAndKey("other");

        bytes memory goodSeller = _sig(escrow.hashAuction(a), sellerKey);
        bytes memory goodPlatform = _sig(escrow.hashProof(p), platformKey);

        bytes memory badPlatform = _sig(escrow.hashProof(p), otherKey);
        bytes memory badSeller = _sig(escrow.hashAuction(a), otherKey);

        vm.prank(winner);
        vm.expectRevert(MarketBase.BadSignature.selector);
        escrow.createFromAuction(a, goodSeller, p, badPlatform);

        vm.prank(winner);
        vm.expectRevert(MarketBase.BadSignature.selector);
        escrow.createFromAuction(a, badSeller, p, goodPlatform);
    }

    function test_revert_settle_auctionsOff() public {
        // signer = 0 disables auctions
        vm.prank(owner);
        config.queueAuctionSigner(address(0));
        vm.warp(block.timestamp + 72 hours);
        config.applyAuctionSigner(address(0));
        Escrow.AuctionAuth memory a = _auth();
        _expectSettleRevert(a, _proof(a, winner, 80 * U), MarketBase.BadSignature.selector);
    }

    function test_settle_auctionOnlyCampaign() public {
        uint8 scope = config.SCOPE_AUCTION();
        vm.prank(owner);
        config.setCampaign(
            EscrowConfig.Campaign({
                start: uint64(block.timestamp),
                end: uint64(block.timestamp + 7 days),
                rateBps: 0,
                scope: scope,
                gasSubsidyCap: 0
            })
        );
        Escrow.AuctionAuth memory a = _auth();
        uint256 id = _settle(a, _proof(a, winner, 80 * U), winner);
        assertEq(escrow.getOrder(id).sellerFee, 0);
        id = _createDefault();
        assertEq(escrow.getOrder(id).sellerFee, 1 * U);
    }

    function test_settle_usdtEmitsSamePriceAndAmount() public {
        Escrow.AuctionAuth memory a = _auth();
        vm.expectEmit(address(escrow));
        emit Escrow.AuctionSettled(1, seller, 42, 0, 0, 80 * U, 80 * U);
        _settle(a, _proof(a, winner, 80 * U), winner);
    }

    // ───── CNY auctions (ADR 0011) ─────

    function test_settle_cny_convertsAtSellerRate() public {
        Escrow.AuctionAuth memory a = _cnyAuth(RATE);
        uint256 w0 = _bal(winner);
        vm.expectEmit(address(escrow));
        emit Escrow.AuctionSettled(1, seller, 42, 1, RATE, 57_600, 80 * U); // 576 CNY at 7.2 = 80 USDT
        uint256 id = _settle(a, _proof(a, winner, 57_600), winner);
        Escrow.Order memory o = escrow.getOrder(id);
        assertEq(o.amount, 80 * U);
        assertEq(o.sellerFee, 1 * U);
        assertEq(w0 - _bal(winner), 80 * U);
    }

    function test_settle_cny_roundsUp() public {
        Escrow.AuctionAuth memory a = _cnyAuth(RATE);
        // 576.01 CNY / 7.2 = 80.0013888… USDT → 80.001389
        uint256 id = _settle(a, _proof(a, winner, 57_601), winner);
        assertEq(escrow.getOrder(id).amount, 80_001_389);
    }

    function test_settle_cny_rateBounds() public {
        Escrow.AuctionAuth memory a = _cnyAuth(49_999);
        _expectSettleRevert(a, _proof(a, winner, 57_600), Escrow.BadOffer.selector);
        a = _cnyAuth(100_001);
        _expectSettleRevert(a, _proof(a, winner, 57_600), Escrow.BadOffer.selector);
        a = _cnyAuth(0);
        _expectSettleRevert(a, _proof(a, winner, 57_600), Escrow.BadOffer.selector);

        a = _cnyAuth(50_000); // 5.0000: 576 CNY = 115.2 USDT
        assertEq(escrow.getOrder(_settle(a, _proof(a, winner, 57_600), winner)).amount, 115_200_000);
        a = _cnyAuth(100_000); // 10.0000: 576 CNY = 57.6 USDT
        a.auctionId = 43;
        assertEq(escrow.getOrder(_settle(a, _proof(a, winner, 57_600), winner)).amount, 57_600_000);
    }

    function test_rateBounds_matchVectors() public view {
        string memory json = vm.readFile(string.concat(vm.projectRoot(), "/../test-vectors/fees.json"));
        assertEq(escrow.MIN_CNY_PER_USDT(), json.readUint(".cny.minCnyPerUsdt"));
        assertEq(escrow.MAX_CNY_PER_USDT(), json.readUint(".cny.maxCnyPerUsdt"));
    }

    function test_revert_settle_badCurrency() public {
        Escrow.AuctionAuth memory a = _auth();
        a.currency = 2;
        _expectSettleRevert(a, _proof(a, winner, 80 * U), Escrow.BadOffer.selector);
        a.cnyPerUsdt = RATE;
        _expectSettleRevert(a, _proof(a, winner, 80 * U), Escrow.BadOffer.selector);

        a = _auth(); // USDT auctions must not carry a rate
        a.cnyPerUsdt = RATE;
        _expectSettleRevert(a, _proof(a, winner, 80 * U), Escrow.BadOffer.selector);
    }

    function test_revert_settle_cny_belowReserveInFen() public {
        Escrow.AuctionAuth memory a = _cnyAuth(RATE);
        _expectSettleRevert(a, _proof(a, winner, 35_999), Escrow.BadOffer.selector);
        assertEq(escrow.getOrder(_settle(a, _proof(a, winner, 36_000), winner)).amount, 50 * U);
    }

    /// Min order (5 USDT) and the per-order cap (200 USDT) apply to the converted amount.
    function test_settle_cny_orderLimitsAfterConversion() public {
        Escrow.AuctionAuth memory a = _cnyAuth(RATE);
        a.reservePrice = 1;
        _expectSettleRevert(a, _proof(a, winner, 3599), Escrow.BadOffer.selector); // 4.999862 USDT
        _expectSettleRevert(a, _proof(a, winner, 144_001), Escrow.BadOffer.selector); // 200.001389 USDT
        assertEq(escrow.getOrder(_settle(a, _proof(a, winner, 144_000), winner)).amount, 200 * U);
        a.auctionId = 43;
        assertEq(escrow.getOrder(_settle(a, _proof(a, winner, 3600), winner)).amount, 5 * U);
    }

    function test_settle_cny_totalLockedCapAfterConversion() public {
        _depositBond(4950 * U);
        Escrow.AuctionAuth memory a = _cnyAuth(RATE);
        _expectSettleRevert(a, _proof(a, winner, 36_001), EscrowConfig.TotalLockedCap.selector); // 50.000139 USDT
        assertEq(escrow.getOrder(_settle(a, _proof(a, winner, 36_000), winner)).amount, 50 * U);
    }

    /// The converted amount never pays the seller less than price ÷ rate, and overpays by under one unit.
    function testFuzz_settle_cny_neverUnderpaysSeller(uint256 priceFen, uint32 rate) public {
        rate = uint32(bound(rate, 50_000, 100_000));
        // keep the converted amount within [5, 200] USDT
        priceFen = bound(priceFen, (5 * U * rate + 1e8 - 1) / 1e8, 200 * U * rate / 1e8);
        Escrow.AuctionAuth memory a = _cnyAuth(rate);
        a.reservePrice = 0;
        uint256 amount = escrow.getOrder(_settle(a, _proof(a, winner, priceFen), winner)).amount;
        assertGe(amount * rate, priceFen * 1e8);
        assertLt((amount - 1) * rate, priceFen * 1e8);
    }

    /// Off-chain signers (viem / alloy) must produce the same digests.
    function test_hashes_matchEip712FieldEncoding() public view {
        Escrow.AuctionAuth memory a = _cnyAuth(RATE);
        bytes32 structHash = keccak256(
            abi.encodePacked(
                escrow.AUCTION_TYPEHASH(),
                abi.encode(a.seller, a.token, a.auctionId, a.reservePrice, a.currency, a.cnyPerUsdt),
                abi.encode(uint8(a.feePayer), a.maxFeeBps),
                abi.encode(a.maxBaseGas, a.maxMinFee, a.instant, a.shipWindow, a.confirmWindow, a.listingHash, a.expiry)
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
        assertEq(escrow.hashAuction(a), keccak256(abi.encodePacked("\x19\x01", domain, structHash)));

        Escrow.SettlementProof memory p = _proof(a, winner, 80 * U);
        bytes32 proofHash = keccak256(abi.encode(escrow.PROOF_TYPEHASH(), p.authHash, p.winner, p.price, p.payDeadline));
        assertEq(escrow.hashProof(p), keccak256(abi.encodePacked("\x19\x01", domain, proofHash)));
    }
}
