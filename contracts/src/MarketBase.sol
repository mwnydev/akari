// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {SignatureCheckerLib} from "solady/utils/SignatureCheckerLib.sol";

/// @title MarketBase — code shared by Escrow and StreamEscrow
/// @notice Signed-offer bookkeeping (fill counts, cancelled nonces, ERC-6492 verification) and pull payments
///         (push first; a refused transfer becomes a credit the account withdraws to any address).
abstract contract MarketBase is ReentrancyGuardTransient {
    using SafeERC20 for IERC20;

    mapping(bytes32 => uint256) public fills;
    mapping(address => mapping(uint256 => bool)) public cancelledNonce;
    mapping(address => mapping(address => uint256)) public credit; // account => token => amount

    event Credited(address indexed account, address indexed token, uint256 amount);
    event Withdrawn(address indexed account, address indexed token, address to, uint256 amount);
    event OfferCancelled(address indexed seller, uint256 nonce);

    error BadSignature();
    error OfferUnavailable();
    error AmountMismatch();
    error NothingToWithdraw();

    /// @notice Withdraw a balance that could not be pushed (e.g. the token froze the recipient) to another address.
    function withdraw(address token, address to) external nonReentrant {
        uint256 amt = credit[msg.sender][token];
        if (amt == 0) revert NothingToWithdraw();
        credit[msg.sender][token] = 0;
        emit Withdrawn(msg.sender, token, to, amt);
        IERC20(token).safeTransfer(to, amt);
    }

    /// @notice Seller voids a signed offer by nonce.
    function cancelOffer(uint256 nonce) external {
        cancelledNonce[msg.sender][nonce] = true;
        emit OfferCancelled(msg.sender, nonce);
    }

    /// @dev Consumes one fill of a seller-signed offer: not expired, not cancelled, fills left, signature valid
    ///      (EOA, ERC-1271 or a not-yet-deployed ERC-6492 account).
    function _useOffer(
        address seller,
        uint256 nonce,
        uint64 expiry,
        uint32 maxFills,
        bytes32 offerHash,
        bytes calldata sig
    ) internal {
        if (block.timestamp > expiry || cancelledNonce[seller][nonce]) revert OfferUnavailable();
        if (fills[offerHash] >= maxFills) revert OfferUnavailable();
        fills[offerHash] += 1;
        if (!SignatureCheckerLib.isValidERC6492SignatureNow(seller, offerHash, sig)) revert BadSignature();
    }

    /// @dev Push payment; if the token refuses (frozen address), keep it claimable via `withdraw(token, to)`.
    function _push(address token, address to, uint256 amt) internal {
        if (amt == 0) return;
        if (!IERC20(token).trySafeTransfer(to, amt)) {
            credit[to][token] += amt;
            emit Credited(to, token, amt);
        }
    }

    /// @dev Pull exactly `amt` from the caller, measured by balance change (never trust the transfer's accounting).
    function _pullExact(address token, uint256 amt) internal {
        uint256 before = IERC20(token).balanceOf(address(this));
        IERC20(token).safeTransferFrom(msg.sender, address(this), amt);
        if (IERC20(token).balanceOf(address(this)) - before != amt) revert AmountMismatch();
    }
}
