// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";
import {MarketBase} from "../src/MarketBase.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FreezableUSDT} from "./mocks/TestTokens.sol";

contract StreamHandler is Test {
    StreamEscrow public stream;
    Escrow public escrow;
    FreezableUSDT public usdt;
    address public arbiter;
    address public feeRecipient;

    address[] public actors; // even index = buyer, odd = seller
    uint256[] internal keys;
    uint256 public totalMinted;
    uint256 internal nonce;
    mapping(uint256 => uint256) public unlockedAtDispute;

    constructor(StreamEscrow s, FreezableUSDT t) {
        stream = s;
        escrow = s.escrow();
        usdt = t;
        arbiter = s.arbiter();
        feeRecipient = s.feeRecipient();
        for (uint256 i; i < 6; i++) {
            (address a, uint256 k) = makeAddrAndKey(string(abi.encodePacked("actor", vm.toString(i))));
            actors.push(a);
            keys.push(k);
            usdt.mint(a, 1_000_000e6);
            totalMinted += 1_000_000e6;
            vm.startPrank(a);
            usdt.approve(address(s), type(uint256).max);
            usdt.approve(address(escrow), type(uint256).max);
            vm.stopPrank();
        }
    }

    function actorCount() external view returns (uint256) {
        return actors.length;
    }

    function _order(uint256 seed) internal view returns (uint256 id, StreamEscrow.Order memory o) {
        uint256 n = stream.orderCount();
        if (n == 0) return (0, o);
        id = bound(seed, 1, n);
        o = stream.getOrder(id);
    }

    function create(uint256 seed, uint256 amount, uint8 payer, uint8 interval, uint32 periods, uint256 bond) external {
        uint256 b = (seed % 3) * 2;
        uint256 amt = bound(amount, 5e6, 200e6);
        StreamEscrow.Interval iv = StreamEscrow.Interval(bound(interval, 0, 3));
        StreamEscrow.Offer memory o = StreamEscrow.Offer({
            seller: actors[b + 1],
            buyer: address(0),
            token: address(usdt),
            amount: amt,
            feePayer: FeeMath.Payer(bound(payer, 0, 2)),
            maxFeeBps: 1000,
            maxBaseGas: 1e6,
            maxClaimGas: 1e6,
            maxMinFee: 5e6,
            bond: seed % 2 == 0 ? 0 : bound(bond, 0, amt),
            lateSlashBps: uint16(seed % 10_001),
            interval: iv,
            periods: uint32(bound(periods, 1, iv == StreamEscrow.Interval.Month ? 12 : 39)),
            startWindow: 3 days,
            listingHash: bytes32(seed),
            expiry: uint64(block.timestamp + 1 days),
            maxFills: 1,
            nonce: ++nonce
        });
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[b + 1], stream.hashOffer(o));
        vm.prank(actors[b]);
        stream.createAndFund(o, abi.encodePacked(r, s, v));
    }

    function depositBond(uint256 seed, uint256 amount) external {
        vm.prank(actors[(seed % 3) * 2 + 1]);
        escrow.depositBond(address(usdt), bound(amount, 1, 300e6));
    }

    function withdrawBond(uint256 seed, uint256 amount) external {
        address s = actors[(seed % 3) * 2 + 1];
        (,, uint256 available) = escrow.bondOf(s, address(usdt));
        vm.prank(s);
        escrow.withdrawBond(address(usdt), bound(amount, 1, available), s);
    }

    function start(uint256 seed) external {
        (uint256 id, StreamEscrow.Order memory o) = _order(seed);
        vm.prank(o.seller);
        stream.start(id);
    }

    function claim(uint256 seed) external {
        (uint256 id, StreamEscrow.Order memory o) = _order(seed);
        vm.prank(o.seller);
        stream.claim(id);
    }

    function finalize(uint256 seed) external {
        (uint256 id,) = _order(seed);
        stream.finalize(id);
    }

    function refund(uint256 seed) external {
        (uint256 id, StreamEscrow.Order memory o) = _order(seed);
        vm.prank(o.seller);
        stream.refund(id);
    }

    function claimTimeout(uint256 seed) external {
        (uint256 id,) = _order(seed);
        stream.claimTimeout(id);
    }

    function extendDeadlines(uint256 seed, uint32 secs) external {
        (uint256 id,) = _order(seed);
        uint256[] memory ids = new uint256[](1);
        ids[0] = id;
        vm.prank(arbiter);
        stream.extendDeadlines(ids, uint32(bound(secs, 1, 7 days)));
    }

    function dispute(uint256 seed, bool bySeller, bool fromBond) external {
        (uint256 id, StreamEscrow.Order memory o) = _order(seed);
        vm.prank(bySeller ? o.seller : o.buyer);
        if (bySeller && fromBond) stream.disputeWithBond(id);
        else stream.dispute(id);
        unlockedAtDispute[id] = stream.unlocked(id);
    }

    function resolve(uint256 seed, uint256 bps, uint256 bondBps) external {
        (uint256 id,) = _order(seed);
        vm.prank(arbiter);
        stream.resolve(id, bound(bps, 0, 10_000), bound(bondBps, 0, 10_000));
    }

    function resolveTimeout(uint256 seed) external {
        (uint256 id,) = _order(seed);
        stream.resolveTimeout(id);
    }

    function propose(uint256 seed, uint256 toBuyer, bool bySeller) external {
        (uint256 id, StreamEscrow.Order memory o) = _order(seed);
        vm.prank(bySeller ? o.seller : o.buyer);
        stream.proposeEnd(id, bound(toBuyer, 0, o.net - o.claimed));
    }

    function accept(uint256 seed) external {
        (uint256 id, StreamEscrow.Order memory o) = _order(seed);
        vm.prank(o.proposer == o.buyer ? o.seller : o.buyer);
        stream.acceptEnd(id, o.proposalToBuyer);
    }

    function warp(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 10 days));
    }

    function toggleFreeze(uint256 seed) external {
        address a = seed % 7 == 6 ? feeRecipient : actors[seed % 6];
        usdt.setFrozen(a, !usdt.frozen(a));
    }

    function withdraw(uint256 seed, bool fromEscrow) external {
        address a = seed % 7 == 6 ? feeRecipient : actors[seed % 6];
        vm.prank(a);
        if (fromEscrow) escrow.withdraw(address(usdt), a);
        else stream.withdraw(address(usdt), a);
    }
}

contract StreamInvariantTest is Test {
    EscrowConfig internal config;
    Escrow internal escrow;
    StreamEscrow internal stream;
    FreezableUSDT internal usdt;
    StreamHandler internal handler;
    address internal feeRecipient = makeAddr("feeRecipient");

    function setUp() public {
        usdt = new FreezableUSDT();
        uint256 n = vm.getNonce(address(this));
        address[] memory markets = new address[](2);
        markets[0] = vm.computeCreateAddress(address(this), n + 1);
        markets[1] = vm.computeCreateAddress(address(this), n + 2);
        config = new EscrowConfig(
            makeAddr("treasurySafe"),
            EscrowConfig.Params({
                feeBps: 100,
                minFee: 1e6,
                minOrder: 5e6,
                orderCap: 200e6,
                totalLockedCap: 1_000_000e6, // the cap itself is unit-tested; here it must not starve the handler
                baseGas: 100_000,
                claimGas: 20_000,
                disputeMulBps: 10_000,
                resolveTimeout: 30 days,
                proposalWindow: 72 hours
            }),
            address(usdt),
            address(0),
            markets
        );
        escrow = new Escrow(config, makeAddr("arbiterSafe"), feeRecipient);
        stream = new StreamEscrow(escrow);
        handler = new StreamHandler(stream, usdt);
        targetContract(address(handler));
    }

    function _open(StreamEscrow.Order memory o) internal pure returns (bool) {
        return o.state == StreamEscrow.State.Funded || o.state == StreamEscrow.State.Disputed;
    }

    function _held(StreamEscrow.Order memory o) internal pure returns (uint256) {
        if (!_open(o)) return 0;
        return uint256(o.net) - o.claimed + o.buyerFee + o.sellerFee + o.buyerGas + o.sellerGas
            + (o.disputedAt != 0 ? o.disputeFee : 0);
    }

    function _sumHeld() internal view returns (uint256 held) {
        for (uint256 id = 1; id <= stream.orderCount(); id++) {
            held += _held(stream.getOrder(id));
        }
    }

    function _sumBonds() internal view returns (uint256 total) {
        for (uint256 i = 1; i < handler.actorCount(); i += 2) {
            (uint256 t,,) = escrow.bondOf(handler.actors(i), address(usdt));
            total += t;
        }
    }

    function _sumCredits(address market) internal view returns (uint256 owed) {
        for (uint256 i; i < handler.actorCount(); i++) {
            owed += MarketBase(market).credit(handler.actors(i), address(usdt));
        }
        owed += MarketBase(market).credit(feeRecipient, address(usdt));
    }

    /// Each market's balance is exactly what it owes: open orders + credits (StreamEscrow), bonds + credits (Escrow).
    function invariant_balancesMatchObligations() public view {
        assertEq(usdt.balanceOf(address(stream)), _sumHeld() + _sumCredits(address(stream)));
        assertEq(usdt.balanceOf(address(escrow)), _sumBonds() + _sumCredits(address(escrow)));
    }

    /// The global risk counter covers both markets: open stream orders + seller bonds.
    function invariant_totalLockedMatches() public view {
        assertEq(config.totalLocked(), _sumHeld() + _sumBonds());
    }

    /// Closed system: tokens only move between buyers, sellers, the fee recipient and the two markets.
    function invariant_conservation() public view {
        uint256 sum = usdt.balanceOf(feeRecipient) + usdt.balanceOf(address(stream)) + usdt.balanceOf(address(escrow));
        for (uint256 i; i < handler.actorCount(); i++) {
            sum += usdt.balanceOf(handler.actors(i));
        }
        assertEq(sum, handler.totalMinted());
    }

    /// Bond locked by StreamEscrow per seller == bonds of that seller's open stream orders (== Escrow's locked).
    function invariant_bondsLockedByOpenOrders() public view {
        for (uint256 i = 1; i < handler.actorCount(); i += 2) {
            address s = handler.actors(i);
            uint256 locked;
            for (uint256 id = 1; id <= stream.orderCount(); id++) {
                StreamEscrow.Order memory o = stream.getOrder(id);
                if (o.seller == s && _open(o)) locked += o.bond;
            }
            (uint256 t, uint256 l,) = escrow.bondOf(s, address(usdt));
            assertEq(escrow.lockedBy(address(stream), s, address(usdt)), locked);
            assertEq(l, locked);
            assertLe(l, t);
        }
    }

    /// claimed ≤ unlocked ≤ net for every open order, and unlocking never grows after a dispute.
    function invariant_unlockBounds() public view {
        for (uint256 id = 1; id <= stream.orderCount(); id++) {
            StreamEscrow.Order memory o = stream.getOrder(id);
            if (!_open(o)) continue;
            uint256 u = stream.unlocked(id);
            assertLe(u, o.net);
            assertLe(o.claimed, u);
            if (o.state == StreamEscrow.State.Disputed) assertEq(u, handler.unlockedAtDispute(id));
        }
    }
}
