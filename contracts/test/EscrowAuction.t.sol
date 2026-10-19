// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {EscrowBase} from "./EscrowBase.sol";

/// Auction settlement (plan §6「拍卖成交」): bids stay off-chain; the winner pays with seller auth + platform proof.
contract EscrowAuctionTest is EscrowBase {
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
        vm.expectRevert(Escrow.OfferUnavailable.selector);
        escrow.createFromAuction(a, sellerSig, p2, platformSig);
    }

    function test_revert_settle_checks() public {
        Escrow.AuctionAuth memory a = _auth();
        Escrow.SettlementProof memory p;

        p = _proof(a, winner, 80 * U);
        p.authHash = keccak256("another auction");
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);

        p = _proof(a, stranger, 80 * U); // caller is not the winner
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);

        p = _proof(a, winner, 50 * U - 1); // below reserve
        _expectSettleRevert(a, p, Escrow.BadOffer.selector);

        p = _proof(a, winner, 80 * U);
        p.payDeadline = uint64(block.timestamp - 1);
        _expectSettleRevert(a, p, Escrow.OfferUnavailable.selector);

        Escrow.AuctionAuth memory expired = _auth();
        expired.expiry = uint64(block.timestamp - 1);
        _expectSettleRevert(expired, _proof(expired, winner, 80 * U), Escrow.OfferUnavailable.selector);

        Escrow.AuctionAuth memory sellerBuys = _auth();
        sellerBuys.seller = winner; // signature check fails first: the winner is not `seller`'s key
        _expectSettleRevert(sellerBuys, _proof(sellerBuys, winner, 80 * U), Escrow.BadSignature.selector);

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
        vm.expectRevert(Escrow.BadSignature.selector);
        escrow.createFromAuction(a, goodSeller, p, badPlatform);

        vm.prank(winner);
        vm.expectRevert(Escrow.BadSignature.selector);
        escrow.createFromAuction(a, badSeller, p, goodPlatform);
    }

    function test_revert_settle_auctionsOff() public {
        // signer = 0 disables auctions
        vm.prank(owner);
        config.queueAuctionSigner(address(0));
        vm.warp(block.timestamp + 72 hours);
        config.applyAuctionSigner(address(0));
        Escrow.AuctionAuth memory a = _auth();
        _expectSettleRevert(a, _proof(a, winner, 80 * U), Escrow.BadSignature.selector);
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

    /// Off-chain signers (viem / alloy) must produce the same digests.
    function test_hashes_matchEip712FieldEncoding() public view {
        Escrow.AuctionAuth memory a = _auth();
        bytes32 structHash = keccak256(
            abi.encodePacked(
                escrow.AUCTION_TYPEHASH(),
                abi.encode(a.seller, a.token, a.auctionId, a.reservePrice, uint8(a.feePayer), a.maxFeeBps),
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
