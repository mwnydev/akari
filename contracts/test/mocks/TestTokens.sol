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

    function decimals() public pure override returns (uint8) {
        return 6;
    }

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

/// @dev Mints 1 extra unit to the receiver of every transfer, so the receiver gets more than `amount`.
contract BonusToken is ERC20 {
    constructor() ERC20("Bonus token", "BON") {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function _update(address from, address to, uint256 value) internal override {
        super._update(from, to, value);
        if (from != address(0) && to != address(0)) super._update(address(0), to, 1);
    }
}

/// @dev 18-decimal token: must be rejected by the token whitelist (all amounts are in 6-decimal units).
contract Token18 is ERC20 {
    constructor() ERC20("Eighteen", "E18") {}
}

/// @dev 5-decimal token: rejected like Token18.
contract Token5 is ERC20 {
    constructor() ERC20("Five", "E5") {}

    function decimals() public pure override returns (uint8) {
        return 5;
    }
}

/// @dev Stand-in for StreamEscrow: a second market that locks and releases seller bonds in Escrow.
interface IBondPool {
    function lockBond(address seller, address token, uint256 amount) external;
    function releaseBond(address seller, address token, uint256 amount, address buyer, uint256 toBuyer) external;
    function takeBond(address seller, address token, uint256 amount) external;
}

contract MockMarket {
    function lock(IBondPool pool, address seller, address token, uint256 amount) external {
        pool.lockBond(seller, token, amount);
    }

    function release(IBondPool pool, address seller, address token, uint256 amount, address buyer, uint256 toBuyer)
        external
    {
        pool.releaseBond(seller, token, amount, buyer, toBuyer);
    }

    function take(IBondPool pool, address seller, address token, uint256 amount) external {
        pool.takeBond(seller, token, amount);
    }
}
