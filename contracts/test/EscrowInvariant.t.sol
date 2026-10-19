// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FreezableUSDT} from "./mocks/TestTokens.sol";

contract EscrowHandler is Test {
    Escrow public escrow;
    FreezableUSDT public usdt;
    address public arbiter;
    address public feeRecipient;

    address[] public actors; // even index = buyer, odd = seller
    uint256[] internal keys;
    mapping(address => uint256) public minted;
    mapping(address => uint256) public paidIn;
    uint256 public totalIn;
    uint256 internal nonce;

    constructor(Escrow e, FreezableUSDT t, address arbiter_, address feeRecipient_) {
        escrow = e;
        usdt = t;
        arbiter = arbiter_;
        feeRecipient = feeRecipient_;
        for (uint256 i; i < 6; i++) {
            (address a, uint256 k) = makeAddrAndKey(string(abi.encodePacked("actor", vm.toString(i))));
            actors.push(a);
            keys.push(k);
            usdt.mint(a, 1_000_000e6);
            minted[a] = 1_000_000e6;
            vm.prank(a);
            usdt.approve(address(e), type(uint256).max);
        }
    }

    function actorCount() external view returns (uint256) {
        return actors.length;
    }

    /// Net tokens an actor moved into the contract (negative flows are covered by balances in the invariant).
    function _pullIn(address from, uint256 before) internal {
        uint256 spent = before - usdt.balanceOf(from);
        paidIn[from] += spent;
        totalIn += spent;
    }

    function _order(uint256 seed) internal view returns (uint256 id, Escrow.Order memory o) {
        uint256 n = escrow.orderCount();
        if (n == 0) return (0, o);
        id = bound(seed, 1, n);
        o = escrow.getOrder(id);
    }

    function create(uint256 seed, uint256 amount, uint8 payer, bool instant, uint256 bond, uint16 slash) external {
        uint256 b = (seed % 3) * 2;
        address buyer = actors[b];
        uint256 amt = bound(amount, 5e6, 200e6);
        Escrow.Offer memory o = Escrow.Offer({
            seller: actors[b + 1],
            buyer: address(0),
            token: address(usdt),
            amount: amt,
            feePayer: FeeMath.Payer(bound(payer, 0, 2)),
            maxFeeBps: 1000,
            maxBaseGas: 1e6,
            maxMinFee: 5e6,
            bond: seed % 2 == 0 ? 0 : bound(bond, 0, amt),
            lateSlashBps: uint16(bound(slash, 0, 10_000)),
            instant: instant,
            shipWindow: 3 days,
            confirmWindow: 7 days,
            listingHash: bytes32(seed),
            expiry: uint64(block.timestamp + 1 days),
            maxFills: 1,
            nonce: ++nonce
        });
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[b + 1], escrow.hashOffer(o));
        uint256 before = usdt.balanceOf(buyer);
        vm.prank(buyer);
        escrow.createAndFund(o, abi.encodePacked(r, s, v));
        _pullIn(buyer, before);
    }

    function depositBond(uint256 seed, uint256 amount) external {
        address s = actors[(seed % 3) * 2 + 1];
        uint256 before = usdt.balanceOf(s);
        vm.prank(s);
        escrow.depositBond(address(usdt), bound(amount, 1, 300e6));
        _pullIn(s, before);
    }

    function withdrawBond(uint256 seed, uint256 amount) external {
        address s = actors[(seed % 3) * 2 + 1];
        (,, uint256 available) = escrow.bondOf(s, address(usdt));
        vm.prank(s);
        escrow.withdrawBond(address(usdt), bound(amount, 1, available), s);
    }

    function deliver(uint256 seed) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        vm.prank(o.seller);
        escrow.markDelivered(id);
    }

    function release(uint256 seed) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        vm.prank(o.buyer);
        escrow.release(id);
    }

    function refund(uint256 seed) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        vm.prank(o.seller);
        escrow.refund(id);
    }

    function claimTimeout(uint256 seed) external {
        (uint256 id,) = _order(seed);
        escrow.claimTimeout(id);
    }

    function extend(uint256 seed, bool bySeller, uint32 secs) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        vm.prank(bySeller ? o.seller : o.buyer);
        escrow.extendConfirm(id, uint32(bound(secs, 1, 10 days)));
    }

    function extendDeadlines(uint256 seed, uint32 secs) external {
        (uint256 id,) = _order(seed);
        uint256[] memory ids = new uint256[](1);
        ids[0] = id;
        vm.prank(arbiter);
        escrow.extendDeadlines(ids, uint32(bound(secs, 1, 7 days)));
    }

    function dispute(uint256 seed, bool bySeller, bool fromBond) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        address who = bySeller ? o.seller : o.buyer;
        uint256 before = usdt.balanceOf(who);
        vm.prank(who);
        if (bySeller && fromBond) escrow.disputeWithBond(id);
        else escrow.dispute(id);
        _pullIn(who, before);
    }

    function resolve(uint256 seed, uint256 bps, uint256 bondBps) external {
        (uint256 id,) = _order(seed);
        vm.prank(arbiter);
        escrow.resolve(id, bound(bps, 0, 10_000), bound(bondBps, 0, 10_000));
    }

    function resolveTimeout(uint256 seed) external {
        (uint256 id,) = _order(seed);
        escrow.resolveTimeout(id);
    }

    function propose(uint256 seed, uint256 toBuyer, bool bySeller) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        vm.prank(bySeller ? o.seller : o.buyer);
        escrow.proposeEnd(id, bound(toBuyer, 0, o.amount));
    }

    function accept(uint256 seed) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        vm.prank(o.proposer == o.buyer ? o.seller : o.buyer);
        escrow.acceptEnd(id, o.proposalToBuyer);
    }

    function warp(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 20 days));
    }

    function toggleFreeze(uint256 seed) external {
        address a = seed % 7 == 6 ? feeRecipient : actors[seed % 6];
        usdt.setFrozen(a, !usdt.frozen(a));
    }

    function withdraw(uint256 seed) external {
        address a = seed % 7 == 6 ? feeRecipient : actors[seed % 6];
        vm.prank(a);
        escrow.withdraw(address(usdt), a);
    }
}

contract EscrowInvariantTest is Test {
    EscrowConfig internal config;
    Escrow internal escrow;
    FreezableUSDT internal usdt;
    EscrowHandler internal handler;
    address internal arbiter = makeAddr("arbiterSafe");
    address internal feeRecipient = makeAddr("feeRecipient");

    function setUp() public {
        usdt = new FreezableUSDT();
        address[] memory markets = new address[](1);
        markets[0] = vm.computeCreateAddress(address(this), vm.getNonce(address(this)) + 1);
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
        escrow = new Escrow(config, arbiter, feeRecipient);
        handler = new EscrowHandler(escrow, usdt, arbiter, feeRecipient);
        targetContract(address(handler));
    }

    function _held(Escrow.Order memory o) internal pure returns (uint256) {
        if (o.state != Escrow.State.Funded && o.state != Escrow.State.Disputed) return 0;
        return uint256(o.amount) + o.buyerFee + o.buyerGas + (o.disputedAt != 0 ? o.disputeFee : 0);
    }

    function _sumHeld() internal view returns (uint256 held) {
        for (uint256 id = 1; id <= escrow.orderCount(); id++) {
            held += _held(escrow.getOrder(id));
        }
    }

    function _sumBonds() internal view returns (uint256 total) {
        for (uint256 i = 1; i < handler.actorCount(); i += 2) {
            (uint256 t,,) = escrow.bondOf(handler.actors(i), address(usdt));
            total += t;
        }
    }

    /// Contract balance is exactly open-order holdings + seller bonds + unclaimed credits.
    function invariant_balanceMatchesObligations() public view {
        uint256 owed = _sumHeld() + _sumBonds();
        for (uint256 i; i < handler.actorCount(); i++) {
            owed += escrow.credit(handler.actors(i), address(usdt));
        }
        owed += escrow.credit(feeRecipient, address(usdt));
        assertEq(usdt.balanceOf(address(escrow)), owed);
    }

    /// The global risk counter tracks exactly what is at risk: open-order holdings + seller bonds.
    function invariant_totalLockedMatches() public view {
        assertEq(config.totalLocked(), _sumHeld() + _sumBonds());
    }

    /// Per seller: locked bond == sum of bonds of that seller's open orders, and locked <= total.
    function invariant_bondsLockedByOpenOrders() public view {
        for (uint256 i = 1; i < handler.actorCount(); i += 2) {
            address s = handler.actors(i);
            uint256 locked;
            for (uint256 id = 1; id <= escrow.orderCount(); id++) {
                Escrow.Order memory o = escrow.getOrder(id);
                if (o.seller == s && (o.state == Escrow.State.Funded || o.state == Escrow.State.Disputed)) {
                    locked += o.bond;
                }
            }
            (uint256 t, uint256 l,) = escrow.bondOf(s, address(usdt));
            assertEq(l, locked);
            assertLe(l, t);
        }
    }

    /// Paid out (to parties and fee recipient) + still held == everything ever paid in.
    /// Money only ever reaches buyers, sellers and the fee recipient.
    function invariant_conservation() public view {
        uint256 out = usdt.balanceOf(feeRecipient);
        for (uint256 i; i < handler.actorCount(); i++) {
            address a = handler.actors(i);
            out += usdt.balanceOf(a) + handler.paidIn(a) - handler.minted(a);
        }
        assertEq(out + usdt.balanceOf(address(escrow)), handler.totalIn());
    }
}
