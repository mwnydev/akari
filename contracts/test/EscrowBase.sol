// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FreezableUSDT} from "./mocks/TestTokens.sol";

abstract contract EscrowBase is Test {
    uint256 internal constant U = 1e6;
    /// A realistic timestamp with many low bits set, so `t + window` and `t | window` differ (forge's default
    /// timestamp of 1 hides that mutation).
    uint256 internal constant START_TIME = 1_799_999_999;

    FreezableUSDT internal usdt;
    EscrowConfig internal config;
    Escrow internal escrow;

    address internal owner = makeAddr("treasurySafe");
    address internal arbiter = makeAddr("arbiterSafe");
    address internal feeRecipient = makeAddr("feeRecipient");
    address internal buyer = makeAddr("buyer");
    address internal stranger = makeAddr("stranger");
    address internal seller;
    uint256 internal sellerKey;
    address internal platform;
    uint256 internal platformKey;

    function setUp() public virtual {
        vm.warp(START_TIME);
        (seller, sellerKey) = makeAddrAndKey("seller");
        (platform, platformKey) = makeAddrAndKey("auctionSigner");
        usdt = new FreezableUSDT();
        (config, escrow) = _deploy(address(usdt), defaultParams());
        _fund(buyer, 1_000_000 * U);
        _fund(seller, 1_000_000 * U);
    }

    /// Same pattern as script/Deploy.s.sol: predict Escrow's address so the config can list it as a market.
    function _deploy(address token, EscrowConfig.Params memory p) internal returns (EscrowConfig c, Escrow e) {
        address predicted = vm.computeCreateAddress(address(this), vm.getNonce(address(this)) + 1);
        address[] memory markets = new address[](1);
        markets[0] = predicted;
        c = new EscrowConfig(owner, p, token, platform, markets);
        e = new Escrow(c, arbiter, feeRecipient);
        assertEq(address(e), predicted);
    }

    /// Plan §9 defaults (single-order cap 200 USDT, total locked cap 5000 USDT).
    function defaultParams() internal pure returns (EscrowConfig.Params memory) {
        return EscrowConfig.Params({
            feeBps: 100,
            minFee: uint64(1 * U),
            minOrder: uint64(5 * U),
            orderCap: uint64(200 * U),
            totalLockedCap: uint64(5000 * U),
            baseGas: uint64(100_000),
            claimGas: uint64(20_000),
            disputeMulBps: 10_000,
            resolveTimeout: 30 days,
            proposalWindow: 72 hours
        });
    }

    function _fund(address who, uint256 amt) internal virtual {
        usdt.mint(who, amt);
        vm.prank(who);
        usdt.approve(address(escrow), type(uint256).max);
    }

    function _offer(uint256 amount, FeeMath.Payer payer) internal view returns (Escrow.Offer memory) {
        return Escrow.Offer({
            seller: seller,
            buyer: address(0),
            token: address(usdt),
            amount: amount,
            feePayer: payer,
            maxFeeBps: 100,
            maxBaseGas: 100_000,
            maxMinFee: 1 * U,
            bond: 0,
            lateSlashBps: 0,
            instant: false,
            shipWindow: 3 days,
            confirmWindow: 7 days,
            listingHash: keccak256("listing"),
            expiry: uint64(block.timestamp + 1 days),
            maxFills: 1,
            nonce: 1
        });
    }

    function _sign(Escrow.Offer memory o, uint256 key) internal view returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, escrow.hashOffer(o));
        return abi.encodePacked(r, s, v);
    }

    function _create(Escrow.Offer memory o) internal returns (uint256 id) {
        bytes memory sig = _sign(o, sellerKey);
        vm.prank(buyer);
        id = escrow.createAndFund(o, sig);
    }

    function _createDefault() internal returns (uint256) {
        return _create(_offer(100 * U, FeeMath.Payer.Seller));
    }

    function _deliver(uint256 id) internal {
        vm.prank(seller);
        escrow.markDelivered(id);
    }

    function _bal(address a) internal view returns (uint256) {
        return usdt.balanceOf(a);
    }

    function _depositBond(uint256 amt) internal {
        vm.prank(seller);
        escrow.depositBond(address(usdt), amt);
    }

    function _bond() internal view returns (uint256 total, uint256 locked, uint256 available) {
        return escrow.bondOf(seller, address(usdt));
    }
}
