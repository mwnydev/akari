// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Escrow} from "../src/Escrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FreezableUSDT} from "./mocks/TestTokens.sol";

abstract contract EscrowBase is Test {
    uint256 internal constant U = 1e6;

    FreezableUSDT internal usdt;
    Escrow internal escrow;

    address internal owner = makeAddr("treasurySafe");
    address internal arbiter = makeAddr("arbiterSafe");
    address internal feeRecipient = makeAddr("feeRecipient");
    address internal buyer = makeAddr("buyer");
    address internal stranger = makeAddr("stranger");
    address internal seller;
    uint256 internal sellerKey;

    function setUp() public virtual {
        (seller, sellerKey) = makeAddrAndKey("seller");
        usdt = new FreezableUSDT();
        escrow = new Escrow(IERC20(address(usdt)), owner, arbiter, feeRecipient, defaultParams());
        _fund(buyer, 1_000_000 * U);
        _fund(seller, 1_000_000 * U);
    }

    function defaultParams() internal pure returns (Escrow.Params memory) {
        return Escrow.Params({
            feeBps: 100,
            campaignBps: 0,
            campaign: false,
            minFee: uint64(1 * U),
            minOrder: uint64(5 * U),
            orderCap: uint64(1000 * U),
            baseGas: uint64(100_000),
            disputeMulBps: 10_000,
            resolveTimeout: 30 days,
            proposalWindow: 72 hours
        });
    }

    function _fund(address who, uint256 amt) internal {
        usdt.mint(who, amt);
        vm.prank(who);
        usdt.approve(address(escrow), type(uint256).max);
    }

    function _offer(uint256 amount, FeeMath.Payer payer) internal view returns (Escrow.Offer memory) {
        return Escrow.Offer({
            seller: seller,
            buyer: address(0),
            amount: amount,
            feePayer: payer,
            maxFeeBps: 100,
            maxBaseGas: 100_000,
            maxMinFee: 1 * U,
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
}
