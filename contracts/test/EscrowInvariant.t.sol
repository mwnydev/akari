// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Escrow} from "../src/Escrow.sol";
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

    function create(uint256 seed, uint256 amount, uint8 payer, bool instant) external {
        uint256 b = (seed % 3) * 2;
        address buyer = actors[b];
        Escrow.Offer memory o = Escrow.Offer({
            seller: actors[b + 1],
            buyer: address(0),
            amount: bound(amount, 5e6, 1000e6),
            feePayer: FeeMath.Payer(bound(payer, 0, 2)),
            maxFeeBps: 1000,
            maxBaseGas: 1e6,
            maxMinFee: 5e6,
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

    function dispute(uint256 seed, bool bySeller) external {
        (uint256 id, Escrow.Order memory o) = _order(seed);
        address who = bySeller ? o.seller : o.buyer;
        uint256 before = usdt.balanceOf(who);
        vm.prank(who);
        escrow.dispute(id);
        _pullIn(who, before);
    }

    function resolve(uint256 seed, uint256 bps) external {
        (uint256 id,) = _order(seed);
        vm.prank(arbiter);
        escrow.resolve(id, bound(bps, 0, 10_000));
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
        escrow.withdraw(a);
    }
}

contract EscrowInvariantTest is Test {
    Escrow internal escrow;
    FreezableUSDT internal usdt;
    EscrowHandler internal handler;
    address internal arbiter = makeAddr("arbiterSafe");
    address internal feeRecipient = makeAddr("feeRecipient");

    function setUp() public {
        usdt = new FreezableUSDT();
        escrow = new Escrow(
            IERC20(address(usdt)),
            makeAddr("treasurySafe"),
            arbiter,
            feeRecipient,
            Escrow.Params({
                feeBps: 100,
                campaignBps: 0,
                campaign: false,
                minFee: 1e6,
                minOrder: 5e6,
                orderCap: 1000e6,
                baseGas: 100_000,
                disputeMulBps: 10_000,
                resolveTimeout: 30 days,
                proposalWindow: 72 hours
            })
        );
        handler = new EscrowHandler(escrow, usdt, arbiter, feeRecipient);
        targetContract(address(handler));
    }

    function _held(Escrow.Order memory o) internal pure returns (uint256) {
        if (o.state != Escrow.State.Funded && o.state != Escrow.State.Disputed) return 0;
        return uint256(o.amount) + o.buyerFee + o.buyerGas + (o.disputedAt != 0 ? o.disputeFee : 0);
    }

    /// Contract balance is exactly what open orders hold plus unclaimed credits — nothing stranded, nothing missing.
    function invariant_balanceMatchesObligations() public view {
        uint256 owed;
        for (uint256 id = 1; id <= escrow.orderCount(); id++) {
            owed += _held(escrow.getOrder(id));
        }
        for (uint256 i; i < handler.actorCount(); i++) {
            owed += escrow.credit(handler.actors(i));
        }
        owed += escrow.credit(feeRecipient);
        assertEq(usdt.balanceOf(address(escrow)), owed);
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
