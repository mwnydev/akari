// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../../src/Escrow.sol";
import {EscrowConfig} from "../../src/EscrowConfig.sol";
import {FeeMath} from "../../src/libraries/FeeMath.sol";
import {SymbolicBase} from "./SymbolicBase.sol";

/// @dev Halmos specs for Escrow (docs/audit/freeze-checklist.md §6). One order per check, every ending covered by its
///      own check so each stays bounded; lifecycle checks take the amount from `_gridAmount` (SymbolicBase). Every
///      ending must succeed when its preconditions hold (no stuck funds), must leave the contract holding only the
///      seller's bond pool, and may only move tokens between the buyer, the seller and the fee recipient.
/// @custom:halmos --solver cvc5-int --solver-timeout-assertion 300000
contract EscrowSymbolic is SymbolicBase {
    // Who opens a dispute: the buyer, the seller paying from the wallet (1), or the seller paying from the bond.
    uint8 internal constant BY_BUYER = 0;
    uint8 internal constant BY_SELLER_BOND = 2;

    struct Terms {
        uint8 size; // `_gridAmount` index
        uint8 payer;
        bool instant;
    }

    /// Offer for `t` with a bond of half the amount (locked, released or slashed by every ending).
    function _offer(Terms memory t) internal view returns (Escrow.Offer memory o) {
        o = _offer(_gridAmount(t.size), t.payer);
        o.bond = o.amount / 2;
        o.instant = t.instant;
    }

    function _offer(uint256 amount, uint8 payer) internal view returns (Escrow.Offer memory) {
        vm.assume(payer <= 2);
        return Escrow.Offer({
            seller: seller,
            buyer: address(0),
            token: address(usdt),
            amount: amount,
            feePayer: FeeMath.Payer(payer),
            maxFeeBps: 1000,
            maxBaseGas: 1 * U,
            maxMinFee: 5 * U,
            bond: 0,
            lateSlashBps: 0,
            instant: false,
            shipWindow: 3 days,
            confirmWindow: 7 days,
            listingHash: bytes32(0),
            expiry: uint64(block.timestamp + 1 days),
            maxFills: 1,
            nonce: 1
        });
    }

    /// Seller deposits the order's bond plus `extraBond` (spare bond to pay a dispute fee from), buyer opens the order.
    function _open(Terms memory t, uint256 extraBond) internal returns (uint256, Escrow.Order memory) {
        return _open(_offer(t), extraBond);
    }

    function _open(Escrow.Offer memory offer, uint256 extraBond) internal returns (uint256 id, Escrow.Order memory o) {
        _depositBond(offer.bond + extraBond);
        vm.prank(BUYER);
        id = escrow.createAndFund(offer, "");
        o = escrow.getOrder(id);
    }

    function _deliver(uint256 id) internal {
        vm.prank(seller);
        escrow.markDelivered(id);
    }

    function _dispute(uint256 id, uint8 by) internal {
        vm.assume(by <= BY_SELLER_BOND);
        vm.prank(by == BY_BUYER ? BUYER : seller);
        if (by == BY_SELLER_BOND) escrow.disputeWithBond(id);
        else escrow.dispute(id);
    }

    function _call(address who, bytes memory data) internal returns (bool ok) {
        vm.prank(who);
        (ok,) = address(escrow).call(data);
    }

    function _pool(Escrow.Order memory o) internal pure returns (uint256) {
        return uint256(o.buyerFee) + o.sellerFee + o.buyerGas + o.sellerGas;
    }

    /// Per order the platform earns base gas plus at most one of {platform fee, dispute fee}.
    function _assertFeeBound(Escrow.Order memory o, uint256 feeRecipientBefore) internal view {
        uint256 fees = uint256(o.buyerFee) + o.sellerFee;
        uint256 maxFee = fees > o.disputeFee ? fees : o.disputeFee;
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeRecipientBefore <= uint256(o.buyerGas) + o.sellerGas + maxFee);
    }

    // ───────────────────────────── Endings ─────────────────────────────

    /// Buyer confirms: the seller gets the principal minus the seller's fee and gas, the fee recipient the whole pool.
    function check_release(Terms memory t, bool delivered) public {
        (uint256 id, Escrow.Order memory o) = _open(t, 0);
        if (delivered && !t.instant) _deliver(id);
        uint256 before = _allowedHoldings();
        uint256 sellerBefore = usdt.balanceOf(seller);
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        assert(_call(BUYER, abi.encodeCall(escrow.release, (id))));
        assert(usdt.balanceOf(seller) - sellerBefore == uint256(o.amount) - o.sellerFee - o.sellerGas);
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore == _pool(o));
        _assertAllSettled(before);
    }

    /// Seller refunds: the buyer gets back everything paid, the platform nothing, the bond is not slashed.
    function check_refund(Terms memory t) public {
        (uint256 id, Escrow.Order memory o) = _open(t, 0);
        uint256 before = _allowedHoldings();
        uint256 buyerBefore = usdt.balanceOf(BUYER);
        uint256 bondBefore = _bondTotal();
        assert(_call(seller, abi.encodeCall(escrow.refund, (id))));
        assert(usdt.balanceOf(BUYER) - buyerBefore == uint256(o.amount) + o.buyerFee + o.buyerGas);
        assert(_bondTotal() == bondBefore);
        _assertAllSettled(before);
    }

    /// Timeout: delivered → release once the confirm window passed; not delivered → full refund plus the agreed share
    /// of the bond once the ship deadline passed.
    function check_claimTimeout(Terms memory t, uint8 bondShare, uint8 slashShare, bool delivered, uint32 late) public {
        Escrow.Offer memory offer = _offer(t);
        offer.bond = _share(offer.amount, bondShare);
        offer.lateSlashBps = _gridBps(slashShare);
        (uint256 id, Escrow.Order memory o) = _open(offer, 0);
        bool shipped = t.instant || delivered;
        if (delivered && !t.instant) _deliver(id);
        o = escrow.getOrder(id);
        vm.warp(uint256(shipped ? o.confirmDeadline : o.shipDeadline) + 1 + late);
        uint256 before = _allowedHoldings();
        uint256 buyerBefore = usdt.balanceOf(BUYER);
        assert(_call(address(0xCAFE), abi.encodeCall(escrow.claimTimeout, (id))));
        if (!shipped) {
            uint256 slash = uint256(o.bond) * o.lateSlashBps / BPS;
            assert(usdt.balanceOf(BUYER) - buyerBefore == uint256(o.amount) + o.buyerFee + o.buyerGas + slash);
        }
        _assertAllSettled(before);
    }

    /// Mutual settlement (`_gridBps` share back to the buyer), optionally after a dispute: always executable,
    /// dispute fee refunded in full.
    function check_acceptEnd(Terms memory t, uint8 buyerShare, bool sellerProposes, bool disputed, uint8 by) public {
        (uint256 id, Escrow.Order memory o) = _open(t, 5 * U);
        uint256 toBuyer = _share(o.amount, buyerShare);
        if (disputed) _dispute(id, by);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        address proposer = sellerProposes ? seller : BUYER;
        address acceptor = sellerProposes ? BUYER : seller;
        assert(_call(proposer, abi.encodeCall(escrow.proposeEnd, (id, toBuyer))));
        assert(_call(acceptor, abi.encodeCall(escrow.acceptEnd, (id, toBuyer))));
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore <= _pool(o));
        _assertAllSettled(before);
    }

    /// Arbiter ruling (`_gridBps` splits): always executable, and the platform keeps at most one of fee / dispute fee.
    function check_resolve(Terms memory t, bool delivered, uint8 by, uint8 split, uint8 bondSplit) public {
        uint256 toBuyerBps = _gridBps(split);
        vm.assume(bondSplit % 2 == 0); // bond ruling: none, half or all
        uint256 bondToBuyerBps = _gridBps(bondSplit);
        (uint256 id, Escrow.Order memory o) = _open(t, 5 * U);
        if (delivered && !t.instant) _deliver(id);
        _dispute(id, by);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        uint256 buyerBefore = usdt.balanceOf(BUYER);
        assert(_call(ARBITER, abi.encodeCall(escrow.resolve, (id, toBuyerBps, bondToBuyerBps))));
        // The buyer receives at least the awarded principal and bond shares, less the seller's base gas when the
        // seller's share is too small to cover it (gas is earned in full once sponsored; freeze checklist L11).
        uint256 awarded = uint256(o.amount) - uint256(o.amount) * (BPS - toBuyerBps) / BPS;
        uint256 uncoveredGas = escrow.getOrder(id).platformPaid ? o.sellerGas : 0;
        assert(usdt.balanceOf(BUYER) - buyerBefore + uncoveredGas >= awarded + uint256(o.bond) * bondToBuyerBps / BPS);
        _assertFeeBound(o, feeBefore);
        _assertAllSettled(before);
    }

    /// Unresolved dispute after the timeout: anyone can settle it; dispute fee refunded, platform fee only.
    function check_resolveTimeout(Terms memory t, bool delivered, uint8 by, uint32 late) public {
        (uint256 id, Escrow.Order memory o) = _open(t, 5 * U);
        if (delivered && !t.instant) _deliver(id);
        _dispute(id, by);
        o = escrow.getOrder(id);
        vm.warp(uint256(o.disputedAt) + o.resolveTimeout + 1 + late);
        uint256 before = _allowedHoldings();
        uint256 feeBefore = usdt.balanceOf(FEE_RECIPIENT);
        assert(_call(address(0xCAFE), abi.encodeCall(escrow.resolveTimeout, (id))));
        assert(usdt.balanceOf(FEE_RECIPIENT) - feeBefore <= _pool(o));
        _assertAllSettled(before);
    }

    // ───────────────────────────── Caps ─────────────────────────────

    /// The per-order cap (200 USDT) and the total-locked cap (5000 USDT) hold after any bond deposit and any two
    /// orders, whatever succeeds. (Dispute fees deliberately bypass the total cap; see the freeze checklist.)
    function check_caps(uint40 bond, uint40 amountA, uint8 payerA, uint40 amountB, uint8 payerB) public {
        vm.prank(seller);
        (bool ok,) = address(escrow).call(abi.encodeCall(escrow.depositBond, (address(usdt), bond)));
        assert(config.totalLocked() <= TOTAL_CAP);
        Escrow.Offer[2] memory offers = [_offer(amountA, payerA), _offer(amountB, payerB)];
        offers[1].nonce = 2;
        for (uint256 i; i < 2; ++i) {
            vm.prank(BUYER);
            (ok,) = address(escrow).call(abi.encodeCall(escrow.createAndFund, (offers[i], "")));
            if (ok) assert(escrow.getOrder(escrow.orderCount()).amount <= ORDER_CAP);
            assert(config.totalLocked() <= TOTAL_CAP);
        }
    }

    /// Whatever the treasury set (within the hard caps) and whatever campaign runs, an order (`_gridAmount`) never
    /// charges more than the seller signed for: fee ≤ max(amount × maxFeeBps, maxMinFee), base gas ≤ maxBaseGas.
    function check_feeWithinSignedCaps(
        EscrowConfig.Params memory p,
        uint16 campaignRate,
        bool campaign,
        uint8 size,
        uint8 payer,
        uint16 maxFeeBps,
        uint40 maxBaseGas,
        uint40 maxMinFee
    ) public {
        p.totalLockedCap = uint64(TOTAL_CAP); // keep `totalLocked` arithmetic small; the cap is checked above
        vm.prank(OWNER);
        config.setParams(p);
        vm.warp(block.timestamp + config.DELAY());
        if (config.pendingEta() != 0) config.applyParams();
        if (campaign) {
            vm.prank(OWNER);
            config.setCampaign(
                EscrowConfig.Campaign({
                    start: uint64(block.timestamp),
                    end: uint64(block.timestamp + 1 days),
                    rateBps: campaignRate,
                    scope: 1,
                    gasSubsidyCap: 0
                })
            );
        }
        uint256 amount = _gridAmount(size);
        Escrow.Offer memory o = _offer(amount, payer);
        o.maxFeeBps = maxFeeBps;
        o.maxBaseGas = maxBaseGas;
        o.maxMinFee = maxMinFee;
        o.expiry = uint64(block.timestamp + 1 days);
        vm.prank(BUYER);
        uint256 id = escrow.createAndFund(o, "");
        Escrow.Order memory r = escrow.getOrder(id);
        uint256 byRate = amount * maxFeeBps / BPS;
        assert(uint256(r.buyerFee) + r.sellerFee <= (byRate > maxMinFee ? byRate : maxMinFee));
        assert(uint256(r.buyerGas) + r.sellerGas <= maxBaseGas);
    }

    // ───────────────────────────── Auction ─────────────────────────────

    function _auth(uint256 reserve, uint8 currency, uint32 rate) internal view returns (Escrow.AuctionAuth memory) {
        return Escrow.AuctionAuth({
            seller: seller,
            token: address(usdt),
            auctionId: 7,
            reservePrice: reserve,
            currency: currency,
            cnyPerUsdt: rate,
            feePayer: FeeMath.Payer.Seller,
            maxFeeBps: 1000,
            maxBaseGas: 1 * U,
            maxMinFee: 5 * U,
            instant: false,
            shipWindow: 3 days,
            confirmWindow: 7 days,
            listingHash: bytes32(0),
            expiry: uint64(block.timestamp + 1 days)
        });
    }

    function _settleAuction(Escrow.AuctionAuth memory a, uint256 price) internal returns (bool ok) {
        Escrow.SettlementProof memory p = Escrow.SettlementProof({
            authHash: escrow.hashAuction(a), winner: BUYER, price: price, payDeadline: uint64(block.timestamp + 1 days)
        });
        vm.prank(BUYER);
        (ok,) = address(escrow).call(abi.encodeCall(escrow.createFromAuction, (a, "", p, "")));
    }

    /// A settled USDT auction met the reserve and charges exactly the price, and the same auction can never settle
    /// again — not even under a different seller authorization (other reserve, currency or rate).
    function check_auction(uint40 reserve, uint40 price, uint40 reserve2, uint40 price2, uint8 currency2, uint32 rate2)
        public
    {
        if (!_settleAuction(_auth(reserve, escrow.CURRENCY_USDT(), 0), price)) return;
        assert(price >= reserve && escrow.getOrder(1).amount == price);
        assert(!_settleAuction(_auth(reserve2, currency2, rate2), price2));
    }

    /// A settled CNY auction met the reserve in fen and pays the seller at least the signed rate implies (ceil,
    /// overpaying by < 1 unit). Rates from a fixed set: with a symbolic rate Solady's `fullMulDivUp` does not finish.
    function check_auction_cny(uint40 reserve, uint40 price, uint8 i) public {
        uint32 rate = _gridRate(i);
        if (!_settleAuction(_auth(reserve, escrow.CURRENCY_CNY(), rate), price)) return;
        assert(price >= reserve);
        uint256 amount = escrow.getOrder(1).amount;
        uint256 target = uint256(price) * FeeMath.CNY_SCALE;
        assert(amount * rate >= target && (amount - 1) * rate < target);
    }
}
