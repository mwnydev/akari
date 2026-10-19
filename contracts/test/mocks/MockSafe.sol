// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

/// @dev The two Safe getters Deploy.s.sol checks on Arbitrum One.
contract MockSafe {
    address[] internal owners;
    uint256 public getThreshold;

    constructor(address[] memory owners_, uint256 threshold_) {
        owners = owners_;
        getThreshold = threshold_;
    }

    function getOwners() external view returns (address[] memory) {
        return owners;
    }
}

/// @dev A contract that is not a Safe at all (calls to the Safe getters revert).
contract NotASafe {}

/// @dev Has `getThreshold` but no `getOwners`.
contract ThresholdOnly {
    uint256 public getThreshold = 2;
}
