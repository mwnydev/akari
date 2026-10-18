// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {MockUSDT} from "../src/MockUSDT.sol";

contract MockUSDTTest is Test {
    function test_decimalsAndMint() public {
        MockUSDT usdt = new MockUSDT();
        assertEq(usdt.decimals(), 6);
        usdt.mint(address(this), 5_000_000);
        assertEq(usdt.balanceOf(address(this)), 5_000_000);
    }
}
