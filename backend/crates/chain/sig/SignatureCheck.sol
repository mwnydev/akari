// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {SignatureCheckerLib} from "solady/utils/SignatureCheckerLib.sol";

/// @title SignatureCheck
/// @notice Off-chain signature check for forum-api (auction authorizations). Never deployed: forum-api sends its
///         creation code plus constructor arguments in an `eth_call` without `to` (the ERC-6492 "deployless" pattern),
///         and the constructor returns the result as 32 bytes instead of runtime code.
/// @dev Uses exactly the check `Escrow.createFromAuction` applies to the seller's signature
///      (`SignatureCheckerLib.isValidERC6492SignatureNow`): ECDSA for EOAs, ERC-1271 for deployed accounts, and for
///      undeployed accounts the ERC-6492 wrapper through Solady's reverting verifier, so nothing it deploys persists.
///      A signature this accepts is one the contract accepts at the same block.
contract SignatureCheck {
    constructor(address signer, bytes32 hash, bytes memory signature) {
        bool valid = SignatureCheckerLib.isValidERC6492SignatureNow(signer, hash, signature);
        assembly {
            mstore(0x00, valid)
            return(0x00, 0x20)
        }
    }
}
