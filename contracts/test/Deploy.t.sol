// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {EntryPoint} from "account-abstraction/core/EntryPoint.sol";
import {Deploy} from "../script/Deploy.s.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {MockUSDT} from "../src/MockUSDT.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";
import {FixedFeePaymaster} from "../src/aa/FixedFeePaymaster.sol";
import {MockSafe, NotASafe, ThresholdOnly} from "./mocks/MockSafe.sol";

contract DeployTest is Test {
    address internal constant VERIFIER = 0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e;
    address internal constant ENTRY_POINT = 0x0000000071727De22E5E9d8BAf0edAc6f37da032;
    uint256 internal constant ARBITRUM_ONE = 42_161;
    Deploy internal script;
    address internal treasury;
    address internal arbiter;
    address internal operations;
    address internal deployer = makeAddr("deployer");

    // Environment variables are process-wide and tests run in parallel, so every test sets the same values: the Safes
    // land at the same addresses in each test's fresh contract. Failure cases call `requireMainnetSafes` directly.
    function setUp() public {
        script = new Deploy();
        treasury = _safe(_owners("siteOwnerDevice"), 2);
        arbiter = _safe(_owners("arbitrationSigner"), 2);
        operations = _safe(_owners("siteOwnerDevice"), 1);
        vm.etch(ENTRY_POINT, address(new EntryPoint()).code);
        vm.setEnv("TOKEN", vm.toString(address(new MockUSDT())));
        vm.setEnv("OWNER", vm.toString(treasury));
        vm.setEnv("ARBITER", vm.toString(arbiter));
        vm.setEnv("FEE_RECIPIENT", vm.toString(treasury));
        vm.setEnv("AUCTION_SIGNER", vm.toString(makeAddr("sponsor")));
        vm.setEnv("PAYMASTER_SIGNER", vm.toString(makeAddr("paymasterSigner")));
        vm.setEnv("PAYMASTER_OWNER", vm.toString(operations));
    }

    function _owners(string memory tag) internal returns (address[] memory o) {
        o = new address[](3);
        for (uint256 i; i < 3; ++i) {
            o[i] = makeAddr(string.concat(tag, vm.toString(i)));
        }
    }

    function _safe(address[] memory owners, uint256 threshold) internal returns (address) {
        return address(new MockSafe(owners, threshold));
    }

    function test_deploy_testnet_deploysVerifier() public {
        vm.chainId(421_614);
        assertEq(VERIFIER.code.length, 0);
        Deploy.Deployment memory d = script.run();
        (EscrowConfig config, Escrow escrow, StreamEscrow stream) = (d.config, d.escrow, d.stream);
        assertGt(VERIFIER.code.length, 0);
        assertTrue(config.isMarket(address(escrow)));
        assertTrue(config.isMarket(address(stream)));
        assertEq(address(stream.escrow()), address(escrow));
        assertEq(address(stream.config()), address(config));
        assertEq(stream.arbiter(), arbiter);
        assertEq(stream.feeRecipient(), treasury);
        assertEq(config.params().claimGas, 20_000);
        assertEq(config.owner(), treasury);
        assertEq(escrow.arbiter(), arbiter);
        assertEq(config.auctionSigner(), makeAddr("sponsor"));
        assertEq(config.params().orderCap, 200e6);
        assertEq(config.params().totalLockedCap, 5000e6);
        assertTrue(config.tokenAllowed(d.token));
        FixedFeePaymaster paymaster = d.paymaster;
        assertEq(address(paymaster.entryPoint()), ENTRY_POINT);
        assertEq(address(paymaster.token()), d.token);
        assertEq(paymaster.treasury(), treasury);
        assertEq(paymaster.signer(), makeAddr("paymasterSigner"));
        assertEq(paymaster.owner(), operations);
    }

    function _inputs() internal returns (Deploy.Inputs memory) {
        return Deploy.Inputs({
            token: address(0),
            owner: treasury,
            arbiter: arbiter,
            feeRecipient: treasury,
            auctionSigner: address(0),
            entryPoint: ENTRY_POINT,
            paymasterSigner: makeAddr("paymasterSigner"),
            paymasterOwner: operations
        });
    }

    function test_deploy_testnet_deploysMockTokenWhenUnset() public {
        vm.chainId(421_614);
        Deploy.Deployment memory d = script.deploy(_inputs());
        assertEq(MockUSDT(d.token).decimals(), 6);
        assertTrue(d.config.tokenAllowed(d.token));
        assertEq(address(d.paymaster.token()), d.token);
    }

    function test_deploy_withoutPaymasterSigner_skipsPaymaster() public {
        vm.chainId(421_614);
        Deploy.Inputs memory c = _inputs();
        c.paymasterSigner = address(0);
        assertEq(address(script.deploy(c).paymaster), address(0));
    }

    function test_deploy_paymasterNeedsOwnerAndEntryPoint() public {
        Deploy.Inputs memory c = _inputs();
        c.paymasterOwner = address(0);
        vm.expectRevert(bytes("PAYMASTER_OWNER required with PAYMASTER_SIGNER"));
        script.deploy(c);
        c = _inputs();
        c.entryPoint = makeAddr("noEntryPoint");
        vm.expectRevert(bytes("EntryPoint missing"));
        script.deploy(c);
    }

    function test_deploy_mainnet_requiresToken() public {
        vm.chainId(ARBITRUM_ONE);
        vm.expectRevert(bytes("TOKEN required on Arbitrum One"));
        script.deploy(_inputs());
    }

    function test_deploy_mainnet_paymasterOwnerMustBeContract() public {
        vm.chainId(ARBITRUM_ONE);
        vm.expectRevert(bytes("PAYMASTER_OWNER has no code (an EOA is not allowed on Arbitrum One)"));
        script.requireMainnetPaymasterOwner(deployer, makeAddr("opsEoa"));
        vm.expectRevert(bytes("PAYMASTER_OWNER must not be the deployer"));
        script.requireMainnetPaymasterOwner(deployer, deployer);
        script.requireMainnetPaymasterOwner(deployer, operations);
    }

    function test_deploy_mainnet_requiresVerifier() public {
        vm.chainId(ARBITRUM_ONE);
        vm.expectRevert(bytes("ERC-6492 verifier missing on Arbitrum One"));
        script.run();
    }

    function test_deploy_mainnet_withSafes() public {
        vm.chainId(ARBITRUM_ONE);
        vm.etch(VERIFIER, hex"00");
        Deploy.Deployment memory d = script.run();
        assertEq(d.config.owner(), treasury);
        assertEq(d.escrow.arbiter(), arbiter);
        assertEq(d.stream.feeRecipient(), treasury);
        assertEq(d.paymaster.owner(), operations);
    }

    function test_deploy_mainnet_checksSafesBeforeDeploying() public {
        vm.chainId(ARBITRUM_ONE);
        vm.etch(VERIFIER, hex"00");
        vm.etch(arbiter, type(NotASafe).runtimeCode);
        vm.expectRevert(bytes("ARBITER is not a Safe (getThreshold failed)"));
        script.run();
    }

    // ───────────── requireMainnetSafes: OWNER / ARBITER / FEE_RECIPIENT must be 2-of-3 Safes ─────────────

    function _check(address owner, address arbiter_, address feeRecipient, string memory reason) internal {
        vm.chainId(ARBITRUM_ONE);
        vm.expectRevert(bytes(reason));
        script.requireMainnetSafes(deployer, owner, arbiter_, feeRecipient);
    }

    function _checkArbiter(address arbiter_, string memory reason) internal {
        _check(treasury, arbiter_, treasury, reason);
    }

    function test_safes_testnetAllowsEoas() public {
        vm.chainId(421_614);
        address eoa = makeAddr("eoa");
        script.requireMainnetSafes(deployer, eoa, deployer, eoa);
    }

    function test_safes_mainnetAcceptsTwoOfThree() public {
        vm.chainId(ARBITRUM_ONE);
        script.requireMainnetSafes(deployer, treasury, arbiter, treasury);
    }

    function test_safes_rejectEoas() public {
        _checkArbiter(makeAddr("arbiterEoa"), "ARBITER has no code (an EOA is not allowed on Arbitrum One)");
        _check(makeAddr("ownerEoa"), arbiter, treasury, "OWNER has no code (an EOA is not allowed on Arbitrum One)");
        _check(
            treasury, arbiter, makeAddr("feeEoa"), "FEE_RECIPIENT has no code (an EOA is not allowed on Arbitrum One)"
        );
    }

    function test_safes_rejectDeployer() public {
        _checkArbiter(deployer, "ARBITER must not be the deployer");
        _check(deployer, arbiter, treasury, "OWNER must not be the deployer");
        address[] memory owners = _owners("arbitrationSigner");
        owners[1] = deployer;
        _checkArbiter(_safe(owners, 2), "ARBITER Safe must not be owned by the deployer");
    }

    function test_safes_rejectArbiterEqualToTreasury() public {
        _checkArbiter(treasury, "ARBITER must not be the treasury Safe");
        _check(treasury, arbiter, arbiter, "ARBITER must not be the treasury Safe");
    }

    function test_safes_rejectWrongThresholdOrOwnerCount() public {
        address[] memory three = _owners("arbitrationSigner");
        _checkArbiter(_safe(three, 1), "ARBITER Safe threshold must be 2");
        _checkArbiter(_safe(three, 3), "ARBITER Safe threshold must be 2");
        address[] memory two = new address[](2);
        (two[0], two[1]) = (three[0], three[1]);
        _checkArbiter(_safe(two, 2), "ARBITER Safe must have exactly 3 owners");
        address[] memory four = new address[](4);
        (four[0], four[1], four[2], four[3]) = (three[0], three[1], three[2], makeAddr("fourth"));
        _check(_safe(four, 2), arbiter, treasury, "OWNER Safe must have exactly 3 owners");
    }

    function test_safes_rejectZeroOrDuplicateOwners() public {
        address[] memory owners = _owners("arbitrationSigner");
        owners[2] = address(0);
        _checkArbiter(_safe(owners, 2), "ARBITER Safe has a zero owner");
        owners[2] = owners[0];
        _checkArbiter(_safe(owners, 2), "ARBITER Safe owners must be distinct");
    }

    function test_safes_rejectNonSafeContracts() public {
        _checkArbiter(address(new NotASafe()), "ARBITER is not a Safe (getThreshold failed)");
        _check(address(new ThresholdOnly()), arbiter, treasury, "OWNER is not a Safe (getOwners failed)");
    }
}
