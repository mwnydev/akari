// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {MockAccountFactory} from "./mocks/MockAccount.sol";
import {EscrowBase} from "./EscrowBase.sol";

/// @notice Sellers are Kernel smart accounts in production: deployed (ERC-1271) or not yet deployed (ERC-6492).
contract EscrowSmartAccountTest is EscrowBase {
    address internal constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    address internal constant REVERTING_VERIFIER = 0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e;
    bytes32 internal constant ERC6492_MAGIC = 0x6492649264926492649264926492649264926492649264926492649264926492;

    MockAccountFactory internal factory;
    bytes32 internal constant SALT = bytes32(uint256(7));

    function setUp() public override {
        super.setUp();
        factory = new MockAccountFactory();
        // Solady's reverting ERC-6492 verifier, deployed permissionlessly via the canonical CREATE2 deployer.
        // Production: must exist on Arbitrum One before launch (deploy script checks / deploys it).
        bytes memory initcode =
            hex"6040600b3d3960403df3fe36383d373d3d6020515160208051013d3d515af160203851516084018038385101606037303452813582523838523490601c34355afa34513060e01b141634fd";
        (bool ok,) = CREATE2_DEPLOYER.call(
            abi.encodePacked(bytes32(0x000000000000000000000000000000000000000068f35e1510740001fd13984a), initcode)
        );
        require(ok && REVERTING_VERIFIER.code.length > 0, "verifier");
    }

    function _accountOffer(address account) internal view returns (Escrow.Offer memory o) {
        o = _offer(100 * U, FeeMath.Payer.Seller);
        o.seller = account;
    }

    function test_deployedAccountSeller_erc1271() public {
        address account = factory.deploy(seller, SALT);
        Escrow.Offer memory o = _accountOffer(account);
        bytes memory sig = _sign(o, sellerKey); // account owner signs the EIP-712 digest
        vm.prank(buyer);
        uint256 id = escrow.createAndFund(o, sig);
        assertEq(escrow.getOrder(id).seller, account);
    }

    function test_deployedAccountSeller_wrongOwnerRejected() public {
        address account = factory.deploy(seller, SALT);
        Escrow.Offer memory o = _accountOffer(account);
        (, uint256 otherKey) = makeAddrAndKey("other");
        bytes memory sig = _sign(o, otherKey);
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadSignature.selector);
        escrow.createAndFund(o, sig);
    }

    function test_undeployedAccountSeller_erc6492() public {
        address account = factory.predict(seller, SALT);
        assertEq(account.code.length, 0);
        Escrow.Offer memory o = _accountOffer(account);
        bytes memory inner = _sign(o, sellerKey);
        bytes memory sig = abi.encodePacked(
            abi.encode(address(factory), abi.encodeCall(MockAccountFactory.deploy, (seller, SALT)), inner),
            ERC6492_MAGIC
        );
        vm.prank(buyer);
        uint256 id = escrow.createAndFund(o, sig);
        assertEq(escrow.getOrder(id).seller, account);
        assertEq(account.code.length, 0, "verification must not deploy the account");

        // The seller can be paid while still undeployed.
        vm.prank(buyer);
        escrow.release(id);
        assertEq(_bal(account), 98_900_000);
    }

    function test_undeployedAccountSeller_forgedRejected() public {
        address account = factory.predict(seller, SALT);
        Escrow.Offer memory o = _accountOffer(account);
        (, uint256 otherKey) = makeAddrAndKey("other");
        bytes memory sig = abi.encodePacked(
            abi.encode(address(factory), abi.encodeCall(MockAccountFactory.deploy, (seller, SALT)), _sign(o, otherKey)),
            ERC6492_MAGIC
        );
        vm.prank(buyer);
        vm.expectRevert(Escrow.BadSignature.selector);
        escrow.createAndFund(o, sig);
    }
}
