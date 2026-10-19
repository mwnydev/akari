// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @dev 6-decimal token that can freeze addresses, like USDT's blacklist.
contract FreezableUSDT is ERC20 {
    mapping(address => bool) public frozen;

    constructor() ERC20("Freezable USDT", "fUSDT") {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function setFrozen(address a, bool f) external {
        frozen[a] = f;
    }

    function _update(address from, address to, uint256 value) internal override {
        require(!frozen[from] && !frozen[to], "frozen");
        super._update(from, to, value);
    }
}

/// @dev Burns 1 unit on every transfer, so the receiver gets less than `amount`.
contract FeeOnTransferToken is ERC20 {
    constructor() ERC20("Fee token", "FOT") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function _update(address from, address to, uint256 value) internal override {
        if (from != address(0) && to != address(0) && value > 0) {
            super._update(from, address(0), 1);
            value -= 1;
        }
        super._update(from, to, value);
    }
}
