// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {Deploy} from "../script/Deploy.s.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {MockUSDT} from "../src/MockUSDT.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";

contract DeployTest is Test {
    address internal constant VERIFIER = 0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e;
    Deploy internal script;

    function setUp() public {
        script = new Deploy();
        vm.setEnv("TOKEN", vm.toString(address(new MockUSDT())));
        vm.setEnv("OWNER", vm.toString(makeAddr("treasurySafe")));
        vm.setEnv("ARBITER", vm.toString(makeAddr("arbiterSafe")));
        vm.setEnv("FEE_RECIPIENT", vm.toString(makeAddr("treasurySafe")));
        vm.setEnv("AUCTION_SIGNER", vm.toString(makeAddr("sponsor")));
    }

    function test_deploy_testnet_deploysVerifier() public {
        vm.chainId(421_614);
        assertEq(VERIFIER.code.length, 0);
        (EscrowConfig config, Escrow escrow, StreamEscrow stream) = script.run();
        assertGt(VERIFIER.code.length, 0);
        assertTrue(config.isMarket(address(escrow)));
        assertTrue(config.isMarket(address(stream)));
        assertEq(address(stream.escrow()), address(escrow));
        assertEq(address(stream.config()), address(config));
        assertEq(stream.arbiter(), makeAddr("arbiterSafe"));
        assertEq(stream.feeRecipient(), makeAddr("treasurySafe"));
        assertEq(config.params().claimGas, 20_000);
        assertEq(config.owner(), makeAddr("treasurySafe"));
        assertEq(escrow.arbiter(), makeAddr("arbiterSafe"));
        assertEq(config.auctionSigner(), makeAddr("sponsor"));
        assertEq(config.params().orderCap, 200e6);
        assertEq(config.params().totalLockedCap, 5000e6);
    }

    function test_deploy_mainnet_requiresVerifier() public {
        vm.chainId(42_161);
        vm.expectRevert(bytes("ERC-6492 verifier missing on Arbitrum One"));
        script.run();
    }
}
