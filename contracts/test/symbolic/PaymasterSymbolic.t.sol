// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {FixedFeePaymaster} from "../../src/aa/FixedFeePaymaster.sol";
import {MockUSDT} from "../../src/MockUSDT.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @dev Only what BasePaymaster's constructor checks; the real EntryPoint v0.7 is exercised by the unit tests.
contract InterfaceOnlyEntryPoint {
    function supportsInterface(bytes4) external pure returns (bool) {
        return true;
    }
}

/// @dev Halmos specs for FixedFeePaymaster. The signature bytes are fully symbolic and Halmos models `ecrecover` as an
///      uninterpreted function, so "some signature recovers to the signer" is always possible: every property below
///      holds even for a sponsor key that signs anything (a stolen key).
/// @custom:halmos --solver cvc5-int --solver-timeout-assertion 300000
contract PaymasterSymbolic is Test {
    address internal constant TREASURY = address(0x7EA5);
    address internal constant SIGNER = address(0x5160);
    address internal constant OPS_SAFE = address(0x0F5);
    address internal constant ACCOUNT = address(0xACC);
    uint256 internal constant START = 1_000_000_000_000; // 1e6 USDT

    address internal entryPoint;
    MockUSDT internal usdt;
    FixedFeePaymaster internal pm;

    function setUp() public {
        entryPoint = address(new InterfaceOnlyEntryPoint());
        usdt = new MockUSDT();
        pm = new FixedFeePaymaster(IEntryPoint(entryPoint), IERC20(address(usdt)), TREASURY, SIGNER, OPS_SAFE);
        usdt.mint(ACCOUNT, START);
        vm.prank(ACCOUNT);
        usdt.approve(address(pm), type(uint256).max);
    }

    function _validate(uint48 validUntil, uint48 validAfter, uint128 fee, bytes32 r, bytes32 s, uint8 v)
        internal
        returns (bool ok, uint256 validationData)
    {
        PackedUserOperation memory op;
        op.sender = ACCOUNT;
        op.paymasterAndData =
            abi.encodePacked(address(pm), uint128(200_000), uint128(0), validUntil, validAfter, fee, r, s, v);
        vm.prank(entryPoint);
        bytes memory ret;
        (ok, ret) = address(pm).call(abi.encodeCall(pm.validatePaymasterUserOp, (op, bytes32(0), 0)));
        if (ok) (, validationData) = abi.decode(ret, (bytes, uint256));
    }

    /// Accepted → exactly the signed fee (≤ 1 USDT) moved from the account to the paymaster, and the window is
    /// bounded (validUntil ≠ 0, ≤ 10 minutes) and reported unchanged to the EntryPoint. Rejected → nothing moved.
    function check_validate(uint48 validUntil, uint48 validAfter, uint128 fee, bytes32 r, bytes32 s, uint8 v) public {
        (bool ok, uint256 vd) = _validate(validUntil, validAfter, fee, r, s, v);
        uint256 taken = START - usdt.balanceOf(ACCOUNT);
        assert(taken <= pm.MAX_FEE());
        assert(usdt.balanceOf(address(pm)) == taken);
        if (!ok || vd & 1 == 1) {
            assert(taken == 0);
            return;
        }
        assert(taken == fee);
        assert(validUntil != 0 && validUntil >= validAfter && validUntil - validAfter <= pm.MAX_VALIDITY());
        assert(uint48(vd >> 160) == validUntil && uint48(vd >> 208) == validAfter);
    }

    /// Only the EntryPoint can trigger validation (and with it the fee pull).
    function check_validate_onlyEntryPoint(address caller, uint128 fee, bytes32 r, bytes32 s, uint8 v) public {
        vm.assume(caller != entryPoint);
        PackedUserOperation memory op;
        op.sender = ACCOUNT;
        op.paymasterAndData = abi.encodePacked(
            address(pm),
            uint128(200_000),
            uint128(0),
            uint48(block.timestamp + 60),
            uint48(block.timestamp),
            fee,
            r,
            s,
            v
        );
        vm.prank(caller);
        (bool ok,) = address(pm).call(abi.encodeCall(pm.validatePaymasterUserOp, (op, bytes32(0), 0)));
        assert(!ok);
        assert(usdt.balanceOf(ACCOUNT) == START);
    }

    /// Collected fees can only go to the immutable treasury, whoever sweeps.
    function check_sweep_onlyToTreasury(address caller, uint128 fee, bytes32 r, bytes32 s, uint8 v) public {
        _validate(uint48(block.timestamp + 60), uint48(block.timestamp), fee, r, s, v);
        uint256 held = usdt.balanceOf(address(pm));
        vm.prank(caller);
        pm.sweepFees();
        assert(usdt.balanceOf(address(pm)) == 0);
        assert(usdt.balanceOf(TREASURY) == held);
        assert(usdt.balanceOf(ACCOUNT) + held == START);
    }

    /// Only the operations Safe rotates the sponsor key.
    function check_setSigner_onlyOwner(address caller, address next) public {
        vm.prank(caller);
        (bool ok,) = address(pm).call(abi.encodeCall(pm.setSigner, (next)));
        if (ok) assert(caller == OPS_SAFE && next != address(0) && pm.signer() == next);
        else assert(pm.signer() == SIGNER);
    }
}
