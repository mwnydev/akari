// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

/// @dev Minimal ERC-1271 smart account owned by one EOA (stands in for a Kernel account).
contract MockAccount {
    address public immutable owner;

    constructor(address owner_) {
        owner = owner_;
    }

    function isValidSignature(bytes32 hash, bytes calldata sig) external view returns (bytes4) {
        (address r,,) = ECDSA.tryRecover(hash, sig);
        return r == owner ? bytes4(0x1626ba7e) : bytes4(0xffffffff);
    }
}

/// @dev CREATE2 factory so a counterfactual (undeployed) account address is known in advance (ERC-6492).
contract MockAccountFactory {
    function deploy(address owner, bytes32 salt) external returns (address a) {
        a = predict(owner, salt);
        if (a.code.length == 0) new MockAccount{salt: salt}(owner);
    }

    function predict(address owner, bytes32 salt) public view returns (address) {
        bytes32 h = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                address(this),
                salt,
                keccak256(abi.encodePacked(type(MockAccount).creationCode, abi.encode(owner)))
            )
        );
        return address(uint160(uint256(h)));
    }
}
