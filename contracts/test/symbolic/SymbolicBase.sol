// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {Escrow} from "../../src/Escrow.sol";
import {EscrowConfig} from "../../src/EscrowConfig.sol";
import {StreamEscrow} from "../../src/StreamEscrow.sol";
import {FreezableUSDT} from "../mocks/TestTokens.sol";

/// @dev ERC-1271 signer that accepts every signature. Signature checking is covered by the unit tests; the symbolic
///      specs only reason about where money can go once an order exists, so any offer may be "signed".
contract AcceptAllSigner {
    function isValidSignature(bytes32, bytes calldata) external pure returns (bytes4) {
        return 0x1626ba7e;
    }
}

/// @dev Value grids shared by the Halmos specs.
abstract contract SymbolicGrids is Test {
    uint256 internal constant BPS = 10_000;

    /// Lifecycle checks draw every amount from small grids: with symbolic amounts the pro-rata fee math
    /// (x * s / amount) and the overflow checks around it are non-linear and the solvers time out. Everything else
    /// (fee payer, who acts, in which order, flags, timestamps) stays symbolic.
    /// Order amounts: the minimum order (minimum fee applies), an odd amount just below the minimum-fee threshold
    /// (1% × 100 USDT = 1 USDT), an odd amount above it, and the order cap. Odd amounts exercise the "half" remainder.
    function _gridAmount(uint8 i) internal pure returns (uint40) {
        vm.assume(i < 4);
        return i == 0 ? 5e6 : i == 1 ? 99_999_999 : i == 2 ? 123_456_789 : 200e6;
    }

    /// Shares (ruling splits, bond and slash sizes, settlement offers): both ends, one unit off each end, and a half.
    function _gridBps(uint8 i) internal pure returns (uint16) {
        vm.assume(i < 5);
        return i == 0 ? 0 : i == 1 ? 1 : i == 2 ? 5000 : i == 3 ? 9999 : 10_000;
    }

    function _share(uint256 amount, uint8 i) internal pure returns (uint256) {
        return amount * _gridBps(i) / BPS;
    }

    /// Seller CNY rates (4 decimals): both hard bounds, a typical rate, an odd one and one unit below the top.
    function _gridRate(uint8 i) internal pure returns (uint32) {
        vm.assume(i < 5);
        return i == 0 ? 50_000 : i == 1 ? 72_000 : i == 2 ? 72_345 : i == 3 ? 99_999 : 100_000;
    }
}

/// @dev Shared Halmos fixture: plan §9 default parameters (order cap 200 USDT, total locked cap 5000 USDT), both
///      markets listed, and only four accounts that ever hold tokens.
abstract contract SymbolicBase is SymbolicGrids {
    uint256 internal constant U = 1e6;
    uint256 internal constant ORDER_CAP = 200 * U;
    uint256 internal constant TOTAL_CAP = 5000 * U;
    uint256 internal constant FUNDS = 1_000_000 * U;

    address internal constant OWNER = address(0x5AFE);
    address internal constant ARBITER = address(0xA4B1);
    address internal constant FEE_RECIPIENT = address(0xFEE);
    address internal constant BUYER = address(0xB0B);

    FreezableUSDT internal usdt;
    EscrowConfig internal config;
    Escrow internal escrow;
    StreamEscrow internal stream;
    address internal seller;
    address internal platform;

    function setUp() public virtual {
        usdt = new FreezableUSDT();
        seller = address(new AcceptAllSigner());
        platform = address(new AcceptAllSigner());
        config = new EscrowConfig(OWNER, defaultParams(), address(usdt), platform, new address[](0));
        escrow = new Escrow(config, ARBITER, FEE_RECIPIENT);
        stream = new StreamEscrow(escrow);
        // Halmos has no `vm.getNonce`, so list the markets through the 72-hour queue instead of predicting addresses.
        vm.startPrank(OWNER);
        config.queueMarket(address(escrow));
        config.queueMarket(address(stream));
        vm.stopPrank();
        vm.warp(block.timestamp + config.DELAY());
        config.addMarket(address(escrow));
        config.addMarket(address(stream));
        _fund(BUYER);
        _fund(seller);
    }

    function defaultParams() internal pure returns (EscrowConfig.Params memory) {
        return EscrowConfig.Params({
            feeBps: 100,
            minFee: uint64(1 * U),
            minOrder: uint64(5 * U),
            orderCap: uint64(ORDER_CAP),
            totalLockedCap: uint64(TOTAL_CAP),
            baseGas: uint64(100_000),
            claimGas: uint64(20_000),
            disputeMulBps: 10_000,
            resolveTimeout: 30 days,
            proposalWindow: 72 hours
        });
    }

    function _fund(address who) internal {
        usdt.mint(who, FUNDS);
        vm.startPrank(who);
        usdt.approve(address(escrow), type(uint256).max);
        usdt.approve(address(stream), type(uint256).max);
        vm.stopPrank();
    }

    /// Tokens held by everyone allowed to hold them. Total supply never changes after setUp, so if this sum is
    /// unchanged, no other address can have received anything.
    function _allowedHoldings() internal view returns (uint256) {
        return usdt.balanceOf(BUYER) + usdt.balanceOf(seller) + usdt.balanceOf(FEE_RECIPIENT)
            + usdt.balanceOf(address(escrow)) + usdt.balanceOf(address(stream));
    }

    function _bondTotal() internal view returns (uint256 total) {
        (total,,) = escrow.bondOf(seller, address(usdt));
    }

    function _bondLocked() internal view returns (uint256 locked) {
        (, locked,) = escrow.bondOf(seller, address(usdt));
    }

    function _depositBond(uint256 amount) internal {
        if (amount == 0) return;
        vm.prank(seller);
        escrow.depositBond(address(usdt), amount);
    }

    /// After every order has ended: the markets hold exactly the seller's bond pool, nothing is locked or owed, and
    /// the custody counter matches.
    function _assertAllSettled(uint256 holdingsBefore) internal view {
        assert(_allowedHoldings() == holdingsBefore);
        uint256 bond = _bondTotal();
        assert(usdt.balanceOf(address(escrow)) == bond);
        assert(usdt.balanceOf(address(stream)) == 0);
        assert(_bondLocked() == 0);
        assert(config.totalLocked() == bond);
    }
}
