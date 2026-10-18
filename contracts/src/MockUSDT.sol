// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @notice TEST-ONLY 6-decimal stand-in for USDT (USD₮0) on Anvil / Arbitrum Sepolia.
/// @dev Anyone can mint. NEVER deploy to Arbitrum One mainnet.
contract MockUSDT is ERC20 {
    constructor() ERC20("Mock USDT (test only)", "mUSDT") {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}
