// SPDX-License-Identifier: GPL-3.0
pragma solidity 0.8.37;

import {EntryPointSimulations} from "account-abstraction/core/EntryPointSimulations.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title SponsorSimulations
/// @notice forum-sponsor's fallback pre-sign simulation for RPC endpoints where `eth_simulateV1` is unavailable
///         (Arbitrum Nitro crashes on it, OffchainLabs/nitro#3372). Never deployed: its runtime code replaces the
///         EntryPoint v0.7 code through an `eth_call` state override (the standard bundler technique), so deposits,
///         nonces, the SenderCreator address and `msg.sender == entryPoint` for factory, paymaster and account are all
///         the real ones.
/// @dev Replays what `eth_simulateV1` replays, using the EntryPoint's own internal steps:
///      1. `_createSenderIfNeeded`: deploy through SenderCreator with `verificationGasLimit` (AA10 / AA13–AA15);
///      2. `_validatePaymasterPrepayment`: deposit check (AA31), `validatePaymasterUserOp` capped at
///         `paymasterVerificationGasLimit` (AA33 / AA36), which pulls the fee; token balances are read around it;
///      3. `sender.call{gas: callGasLimit}(callData)`, as `innerHandleOp` does for non-`executeUserOp` callData
///         (forum-sponsor only signs Kernel `execute`).
///      Account signature validation is skipped (the user has not signed yet); the relayer's `handleOps` dry run covers
///      it before submission. The caller overrides the paymaster's `signer` slot with a throwaway key, so no real
///      sponsor signature is ever sent to the RPC provider.
///      Storage: `deposits` (slot 0) and `nonceSequenceNumber` (slot 1) line up with the deployed EntryPoint v0.7
///      (checked on an Arbitrum Sepolia fork); the simulation-only slots after them are written only inside the call.
contract SponsorSimulations is EntryPointSimulations {
    struct Outcome {
        uint256 deployGas;
        uint256 paymasterValidationData;
        uint256 senderBalanceBefore;
        uint256 senderBalanceAfter;
        uint256 paymasterBalanceBefore;
        uint256 paymasterBalanceAfter;
        bool executed;
        uint256 executionGas;
        bytes executionResult;
    }

    error NoPaymaster();
    error PaymasterContext();

    function simulateSponsored(PackedUserOperation calldata op, IERC20 token) external returns (Outcome memory out) {
        initSenderCreator();
        UserOpInfo memory info;
        _copyUserOpToMemory(op, info.mUserOp);
        info.userOpHash = getUserOpHash(op);
        address sender = info.mUserOp.sender;
        address paymaster = info.mUserOp.paymaster;
        if (paymaster == address(0)) revert NoPaymaster();
        // Same bound as `_validatePrepayment`, so the prefund product below cannot overflow.
        uint256 maxGasValues = info.mUserOp.preVerificationGas | info.mUserOp.verificationGasLimit
            | info.mUserOp.callGasLimit | info.mUserOp.paymasterVerificationGasLimit
            | info.mUserOp.paymasterPostOpGasLimit | info.mUserOp.maxFeePerGas | info.mUserOp.maxPriorityFeePerGas;
        require(maxGasValues <= type(uint120).max, "AA94 gas values overflow");

        uint256 gas = gasleft();
        _createSenderIfNeeded(0, info, op.initCode);
        out.deployGas = gas - gasleft();

        out.senderBalanceBefore = token.balanceOf(sender);
        out.paymasterBalanceBefore = token.balanceOf(paymaster);
        (bytes memory context, uint256 validationData) =
            _validatePaymasterPrepayment(0, op, info, _getRequiredPrefund(info.mUserOp));
        // FixedFeePaymaster has no postOp; a context would mean a different paymaster than the one we sign for.
        if (context.length != 0) revert PaymasterContext();
        out.paymasterValidationData = validationData;
        out.senderBalanceAfter = token.balanceOf(sender);
        out.paymasterBalanceAfter = token.balanceOf(paymaster);

        gas = gasleft();
        (out.executed, out.executionResult) = sender.call{gas: info.mUserOp.callGasLimit}(op.callData);
        out.executionGas = gas - gasleft();
    }
}
