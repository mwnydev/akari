// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {BasePaymaster} from "account-abstraction/core/BasePaymaster.sol";
import {UserOperationLib} from "account-abstraction/core/UserOperationLib.sol";
import {_packValidationData} from "account-abstraction/core/Helpers.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/// @title FixedFeePaymaster
/// @notice EntryPoint v0.7 paymaster that sponsors gas in ETH and charges a fixed USDT fee taken during validation
///         (docs/adr/0004). The fee is pulled before the account's callData runs, so it is collected even when
///         callData reverts, and callData cannot move the funds out first.
/// @dev paymasterAndData layout (after the standard 52-byte prefix of address + two uint128 gas limits):
///      [52:58] validUntil (uint48) | [58:64] validAfter (uint48) | [64:80] fee (uint128) | [80:] signer signature.
///      The signature covers every userOp field except the two signatures, plus chainid, this address and the three
///      values above (same fields as eth-infinitism VerifyingPaymaster v0.7, plus the fee).
///
///      ERC-7562 storage rules (relevant only when ops go through a public bundler; the private relayer does not
///      enforce them): the paymaster must be staked. The fee transfer touches token storage keyed by the sender
///      (balance, allowance[sender]) and by this paymaster (balance[this]); both are allowed for a staked entity.
///      Writing the treasury's balance would NOT be allowed, so fees accrue here and `sweepFees` forwards them to the
///      immutable treasury. Upgradeable tokens (USD₮0 is a proxy) also read their implementation slot; verify the
///      fallback bundler accepts this before relying on it.
contract FixedFeePaymaster is BasePaymaster {
    using SafeERC20 for IERC20;
    using UserOperationLib for PackedUserOperation;

    /// @notice Hard cap on the per-operation fee: 1 USDT (6 decimals). Not configurable (plan section 9).
    uint256 public constant MAX_FEE = 1_000_000;

    /// @notice Longest sponsorship window (validUntil - validAfter). forum-sponsor signs for 2 minutes; the extra room
    ///         covers clock skew. A window is required because validation may not read block.timestamp (ERC-7562),
    ///         so the sponsor must set validAfter to its current time.
    uint256 public constant MAX_VALIDITY = 10 minutes;

    uint256 private constant VALID_UNTIL_OFFSET = 52;
    uint256 private constant VALID_AFTER_OFFSET = 58;
    uint256 private constant FEE_OFFSET = 64;
    uint256 private constant SIGNATURE_OFFSET = 80;

    IERC20 public immutable token;
    address public immutable treasury;
    address public signer;

    event SignerChanged(address indexed previous, address indexed current);
    event FeeCharged(address indexed sender, uint256 fee);
    event FeesSwept(uint256 amount);

    error ZeroAddress();
    error FeeAboveCap(uint256 fee);
    error MalformedPaymasterData();

    constructor(IEntryPoint entryPoint_, IERC20 token_, address treasury_, address signer_, address owner_)
        BasePaymaster(entryPoint_)
    {
        if (address(token_) == address(0) || treasury_ == address(0) || signer_ == address(0) || owner_ == address(0)) {
            revert ZeroAddress();
        }
        token = token_;
        treasury = treasury_;
        signer = signer_;
        emit SignerChanged(address(0), signer_);
        _transferOwnership(owner_);
    }

    /// @notice Rotate the off-chain sponsor key (forum-sponsor). Only the operations Safe.
    function setSigner(address signer_) external onlyOwner {
        if (signer_ == address(0)) revert ZeroAddress();
        emit SignerChanged(signer, signer_);
        signer = signer_;
    }

    /// @notice Forward accumulated fees to the treasury. Anyone may call; the destination is fixed.
    function sweepFees() external {
        uint256 amount = token.balanceOf(address(this));
        emit FeesSwept(amount);
        token.safeTransfer(treasury, amount);
    }

    /// @notice Hash the sponsor signs for `userOp`: the userOp fields (minus signatures) hashed first, then bound to
    ///         the chain, this paymaster and the sponsorship terms. Split in two to stay within the EVM stack limit.
    function getHash(PackedUserOperation calldata userOp, uint48 validUntil, uint48 validAfter, uint128 fee)
        public
        view
        returns (bytes32)
    {
        return keccak256(abi.encode(_opHash(userOp), block.chainid, address(this), validUntil, validAfter, fee));
    }

    function _opHash(PackedUserOperation calldata userOp) private pure returns (bytes32) {
        return keccak256(
            abi.encode(
                userOp.getSender(),
                userOp.nonce,
                keccak256(userOp.initCode),
                keccak256(userOp.callData),
                userOp.accountGasLimits,
                uint256(
                    bytes32(
                        userOp.paymasterAndData[UserOperationLib.PAYMASTER_VALIDATION_GAS_OFFSET:UserOperationLib.PAYMASTER_DATA_OFFSET

                        ]
                    )
                ),
                userOp.preVerificationGas,
                userOp.gasFees
            )
        );
    }

    function _validatePaymasterUserOp(PackedUserOperation calldata userOp, bytes32, uint256)
        internal
        override
        returns (bytes memory context, uint256 validationData)
    {
        bytes calldata data = userOp.paymasterAndData;
        if (data.length < SIGNATURE_OFFSET) revert MalformedPaymasterData();
        uint48 validUntil = uint48(bytes6(data[VALID_UNTIL_OFFSET:VALID_AFTER_OFFSET]));
        uint48 validAfter = uint48(bytes6(data[VALID_AFTER_OFFSET:FEE_OFFSET]));
        uint128 fee = uint128(bytes16(data[FEE_OFFSET:SIGNATURE_OFFSET]));
        if (fee > MAX_FEE) revert FeeAboveCap(fee);

        bytes32 hash = MessageHashUtils.toEthSignedMessageHash(getHash(userOp, validUntil, validAfter, fee));
        (address recovered, ECDSA.RecoverError err,) = ECDSA.tryRecoverCalldata(hash, data[SIGNATURE_OFFSET:]);
        bool badWindow = validUntil == 0 || validUntil < validAfter || validUntil - validAfter > MAX_VALIDITY;
        if (badWindow || err != ECDSA.RecoverError.NoError || recovered != signer) {
            // Signature failure (or an unbounded / over-long window) is reported, not reverted, per ERC-4337;
            // no fee is taken.
            return ("", _packValidationData(true, validUntil, validAfter));
        }

        if (fee > 0) {
            address sender = userOp.getSender();
            emit FeeCharged(sender, fee);
            // `sender` is the userOp's own account. If its signature fails, the EntryPoint reverts the whole
            // handleOps (AA24) after this call, undoing the transfer; it only pulls the allowance the account
            // granted to this paymaster. See SECURITY.md.
            // slither-disable-next-line arbitrary-send-erc20
            token.safeTransferFrom(sender, address(this), fee);
        }
        return ("", _packValidationData(false, validUntil, validAfter));
    }
}
