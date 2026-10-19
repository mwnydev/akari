// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Script, console} from "forge-std/Script.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";

/// @dev The two Safe getters the mainnet checks read.
interface ISafe {
    function getOwners() external view returns (address[] memory);
    function getThreshold() external view returns (uint256);
}

/// @notice Deploys EscrowConfig + Escrow + StreamEscrow (both markets registered at construction). Env:
///   TOKEN           USD₮0 on Arbitrum One (MockUSDT on testnet)
///   OWNER           treasury Safe (config owner; it must `acceptOwnership` only if transferred later)
///   ARBITER         arbitration Safe (2/3)
///   FEE_RECIPIENT   treasury Safe
///   AUCTION_SIGNER  forum-sponsor auction key (optional; unset = auctions off)
/// On Arbitrum One OWNER, ARBITER and FEE_RECIPIENT must be 2-of-3 Safes (ADR 0013 §5, plan §9); testnets and local
/// chains may use EOAs.
/// Usage: forge script script/Deploy.s.sol --rpc-url arbitrum_sepolia --broadcast --verify
contract Deploy is Script {
    address internal constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    /// Solady's reverting ERC-6492 verifier; `SignatureCheckerLib.isValidERC6492SignatureNow` calls it to check
    /// signatures from smart accounts that are not deployed yet.
    address internal constant ERC6492_VERIFIER = 0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e;
    bytes32 internal constant VERIFIER_SALT = 0x000000000000000000000000000000000000000068f35e1510740001fd13984a;
    bytes internal constant VERIFIER_INITCODE =
        hex"6040600b3d3960403df3fe36383d373d3d6020515160208051013d3d515af160203851516084018038385101606037303452813582523838523490601c34355afa34513060e01b141634fd";
    uint256 internal constant ARBITRUM_ONE = 42_161;
    uint256 internal constant U = 1e6;
    uint256 internal constant SAFE_THRESHOLD = 2;
    uint256 internal constant SAFE_OWNERS = 3;

    /// Plan §9 defaults; launch caps: 200 USDT per order, 5000 USDT held in total.
    function defaultParams() public pure returns (EscrowConfig.Params memory) {
        return EscrowConfig.Params({
            feeBps: 100,
            minFee: uint64(1 * U),
            minOrder: uint64(5 * U),
            orderCap: uint64(200 * U),
            totalLockedCap: uint64(5000 * U),
            baseGas: uint64(100_000),
            claimGas: uint64(20_000),
            disputeMulBps: 10_000,
            resolveTimeout: 30 days,
            proposalWindow: 72 hours
        });
    }

    function run() external returns (EscrowConfig config, Escrow escrow, StreamEscrow stream) {
        address token = vm.envAddress("TOKEN");
        address owner = vm.envAddress("OWNER");
        address arbiter = vm.envAddress("ARBITER");
        address feeRecipient = vm.envAddress("FEE_RECIPIENT");
        address auctionSigner = vm.envOr("AUCTION_SIGNER", address(0));

        vm.startBroadcast();
        (, address deployer,) = vm.readCallers();
        requireMainnetSafes(deployer, owner, arbiter, feeRecipient);
        _ensureVerifier();
        uint256 nonce = vm.getNonce(deployer);
        address[] memory markets = new address[](2);
        markets[0] = vm.computeCreateAddress(deployer, nonce + 1);
        markets[1] = vm.computeCreateAddress(deployer, nonce + 2);
        config = new EscrowConfig(owner, defaultParams(), token, auctionSigner, markets);
        escrow = new Escrow(config, arbiter, feeRecipient);
        stream = new StreamEscrow(escrow);
        vm.stopBroadcast();

        require(address(escrow) == markets[0], "escrow address mismatch");
        require(address(stream) == markets[1], "stream address mismatch");
        console.log("EscrowConfig", address(config));
        console.log("Escrow      ", address(escrow));
        console.log("StreamEscrow", address(stream));
        console.log("verifier    ", ERC6492_VERIFIER);
    }

    /// Arbitrum One: a typo'd address or a personal EOA here would hand the escrowed funds' arbitration or the fee
    /// settings to one key, so refuse anything but the planned 2-of-3 Safes. The arbitration Safe (2 admins + site
    /// owner) must not be the treasury Safe (the site owner's 3 devices).
    function requireMainnetSafes(address deployer, address owner, address arbiter, address feeRecipient) public view {
        if (block.chainid != ARBITRUM_ONE) return;
        require(arbiter != owner && arbiter != feeRecipient, "ARBITER must not be the treasury Safe");
        _requireSafe("ARBITER", arbiter, deployer);
        _requireSafe("OWNER", owner, deployer);
        _requireSafe("FEE_RECIPIENT", feeRecipient, deployer);
    }

    function _requireSafe(string memory name, address safe, address deployer) internal view {
        require(safe != deployer, string.concat(name, " must not be the deployer"));
        require(safe.code.length > 0, string.concat(name, " has no code (an EOA is not allowed on Arbitrum One)"));
        uint256 threshold;
        address[] memory owners;
        try ISafe(safe).getThreshold() returns (uint256 t) {
            threshold = t;
        } catch {
            revert(string.concat(name, " is not a Safe (getThreshold failed)"));
        }
        try ISafe(safe).getOwners() returns (address[] memory o) {
            owners = o;
        } catch {
            revert(string.concat(name, " is not a Safe (getOwners failed)"));
        }
        require(threshold == SAFE_THRESHOLD, string.concat(name, " Safe threshold must be 2"));
        require(owners.length == SAFE_OWNERS, string.concat(name, " Safe must have exactly 3 owners"));
        for (uint256 i; i < owners.length; ++i) {
            address o = owners[i];
            require(o != address(0), string.concat(name, " Safe has a zero owner"));
            require(o != deployer, string.concat(name, " Safe must not be owned by the deployer"));
            for (uint256 j; j < i; ++j) {
                require(o != owners[j], string.concat(name, " Safe owners must be distinct"));
            }
        }
    }

    /// Mainnet: the verifier must already exist (deploying it is a deliberate, separate step). Testnets and local
    /// chains: deploy it through the canonical CREATE2 deployer, which yields the same address everywhere.
    function _ensureVerifier() internal {
        if (ERC6492_VERIFIER.code.length > 0) return;
        require(block.chainid != ARBITRUM_ONE, "ERC-6492 verifier missing on Arbitrum One");
        (bool ok,) = CREATE2_DEPLOYER.call(abi.encodePacked(VERIFIER_SALT, VERIFIER_INITCODE));
        require(ok && ERC6492_VERIFIER.code.length > 0, "verifier deploy failed");
    }
}
