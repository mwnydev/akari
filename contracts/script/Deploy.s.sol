// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Script, console} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {MockUSDT} from "../src/MockUSDT.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";
import {FixedFeePaymaster} from "../src/aa/FixedFeePaymaster.sol";

/// @dev The two Safe getters the mainnet checks read.
interface ISafe {
    function getOwners() external view returns (address[] memory);
    function getThreshold() external view returns (uint256);
}

/// @notice Deploys EscrowConfig + Escrow + StreamEscrow (both markets registered at construction) and, when
///         PAYMASTER_SIGNER is set, FixedFeePaymaster. Env:
///   TOKEN             USD₮0 on Arbitrum One; unset on testnets / local chains = deploy MockUSDT first
///   OWNER             treasury Safe (config owner; it must `acceptOwnership` only if transferred later)
///   ARBITER           arbitration Safe (2/3)
///   FEE_RECIPIENT     treasury Safe (also the paymaster's immutable fee treasury)
///   AUCTION_SIGNER    forum-sponsor auction key (optional; unset = auctions off)
///   PAYMASTER_SIGNER  forum-sponsor paymaster key (optional; unset = no paymaster)
///   PAYMASTER_OWNER   operations Safe (paymaster owner: stake, signer rotation, deposit withdrawal)
/// On Arbitrum One OWNER, ARBITER, FEE_RECIPIENT and PAYMASTER_OWNER must be 2-of-3 Safes (ADR 0013 §5, plan §9);
/// testnets and local chains may use EOAs or threshold-1 Safes.
/// Usage: forge script script/Deploy.s.sol --rpc-url arbitrum_sepolia --broadcast --verify (or deploy/testnet/testnet.sh)
contract Deploy is Script {
    struct Inputs {
        address token;
        address owner;
        address arbiter;
        address feeRecipient;
        address auctionSigner;
        address entryPoint;
        address paymasterSigner;
        address paymasterOwner;
    }

    struct Deployment {
        address token;
        EscrowConfig config;
        Escrow escrow;
        StreamEscrow stream;
        FixedFeePaymaster paymaster; // zero when PAYMASTER_SIGNER is unset
    }

    address internal constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    /// Solady's reverting ERC-6492 verifier; `SignatureCheckerLib.isValidERC6492SignatureNow` calls it to check
    /// signatures from smart accounts that are not deployed yet.
    address internal constant ERC6492_VERIFIER = 0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e;
    bytes32 internal constant VERIFIER_SALT = 0x000000000000000000000000000000000000000068f35e1510740001fd13984a;
    bytes internal constant VERIFIER_INITCODE =
        hex"6040600b3d3960403df3fe36383d373d3d6020515160208051013d3d515af160203851516084018038385101606037303452813582523838523490601c34355afa34513060e01b141634fd";
    /// EntryPoint v0.7 (ADR 0003), the same address on every chain.
    address internal constant ENTRY_POINT = 0x0000000071727De22E5E9d8BAf0edAc6f37da032;
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

    function run() external returns (Deployment memory) {
        return deploy(
            Inputs({
                token: vm.envOr("TOKEN", address(0)),
                owner: vm.envAddress("OWNER"),
                arbiter: vm.envAddress("ARBITER"),
                feeRecipient: vm.envAddress("FEE_RECIPIENT"),
                auctionSigner: vm.envOr("AUCTION_SIGNER", address(0)),
                entryPoint: ENTRY_POINT,
                paymasterSigner: vm.envOr("PAYMASTER_SIGNER", address(0)),
                paymasterOwner: vm.envOr("PAYMASTER_OWNER", address(0))
            })
        );
    }

    function deploy(Inputs memory c) public returns (Deployment memory d) {
        bool withPaymaster = c.paymasterSigner != address(0);
        require(!withPaymaster || c.paymasterOwner != address(0), "PAYMASTER_OWNER required with PAYMASTER_SIGNER");
        require(!withPaymaster || c.entryPoint.code.length > 0, "EntryPoint missing");

        vm.startBroadcast();
        (, address deployer,) = vm.readCallers();
        requireMainnetSafes(deployer, c.owner, c.arbiter, c.feeRecipient);
        if (withPaymaster) requireMainnetPaymasterOwner(deployer, c.paymasterOwner);
        if (c.token == address(0)) {
            require(block.chainid != ARBITRUM_ONE, "TOKEN required on Arbitrum One");
            c.token = address(new MockUSDT());
        }
        d.token = c.token;
        _ensureVerifier();
        uint256 nonce = vm.getNonce(deployer);
        address[] memory markets = new address[](2);
        markets[0] = vm.computeCreateAddress(deployer, nonce + 1);
        markets[1] = vm.computeCreateAddress(deployer, nonce + 2);
        d.config = new EscrowConfig(c.owner, defaultParams(), c.token, c.auctionSigner, markets);
        d.escrow = new Escrow(d.config, c.arbiter, c.feeRecipient);
        d.stream = new StreamEscrow(d.escrow);
        if (withPaymaster) {
            d.paymaster = new FixedFeePaymaster(
                IEntryPoint(c.entryPoint), IERC20(c.token), c.feeRecipient, c.paymasterSigner, c.paymasterOwner
            );
        }
        vm.stopBroadcast();

        require(address(d.escrow) == markets[0], "escrow address mismatch");
        require(address(d.stream) == markets[1], "stream address mismatch");
        console.log("Token            ", d.token);
        console.log("EscrowConfig     ", address(d.config));
        console.log("Escrow           ", address(d.escrow));
        console.log("StreamEscrow     ", address(d.stream));
        console.log("FixedFeePaymaster", address(d.paymaster));
        console.log("verifier         ", ERC6492_VERIFIER);
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

    /// Arbitrum One: the paymaster owner (operations Safe) can withdraw the ETH deposit and rotate the sponsor key, so
    /// it gets the same 2-of-3 checks as the other Safes. Testnets keep a threshold-1 operations Safe.
    function requireMainnetPaymasterOwner(address deployer, address paymasterOwner) public view {
        if (block.chainid != ARBITRUM_ONE) return;
        _requireSafe("PAYMASTER_OWNER", paymasterOwner, deployer);
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
