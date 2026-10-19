// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {EntryPoint} from "account-abstraction/core/EntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/// @dev Shared helpers for tests against a real EntryPoint v0.7.
abstract contract AATestBase is Test {
    EntryPoint internal entryPoint;
    address payable internal bundler = payable(makeAddr("bundler"));

    uint128 internal constant VERIFICATION_GAS = 500_000;
    uint128 internal constant CALL_GAS = 300_000;
    uint128 internal constant PM_VERIFICATION_GAS = 200_000;
    uint256 internal constant PRE_VERIFICATION_GAS = 50_000;
    uint128 internal constant GAS_PRICE = 1 gwei;

    function _deployEntryPoint() internal {
        entryPoint = new EntryPoint();
    }

    function _baseOp(address sender, bytes memory callData) internal view returns (PackedUserOperation memory op) {
        op.sender = sender;
        op.nonce = entryPoint.getNonce(sender, 0);
        op.callData = callData;
        op.accountGasLimits = bytes32((uint256(VERIFICATION_GAS) << 128) | CALL_GAS);
        op.preVerificationGas = PRE_VERIFICATION_GAS;
        op.gasFees = bytes32((uint256(GAS_PRICE) << 128) | GAS_PRICE);
    }

    /// @dev Raw ECDSA over userOpHash with the eth-signed prefix (SimpleAccount v0.7 scheme).
    function _signEthPrefixed(uint256 key, bytes32 hash) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, MessageHashUtils.toEthSignedMessageHash(hash));
        return abi.encodePacked(r, s, v);
    }

    function _handle(PackedUserOperation memory op) internal {
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.prank(bundler, bundler);
        entryPoint.handleOps(ops, bundler);
    }
}
