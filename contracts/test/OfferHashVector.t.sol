// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {console2} from "forge-std/console2.sol";
import {Escrow} from "../src/Escrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {EscrowBase} from "./EscrowBase.sol";

/// Shared vector for the backend's EIP-712 offer hash (backend/crates/core/src/eip712.rs).
/// Both sides assert the same constants; `-vv` prints them when the encoding changes on purpose.
contract OfferHashVectorTest is EscrowBase {
    address internal constant AT = 0x4444444444444444444444444444444444444444;

    bytes32 internal constant TYPEHASH = 0x04f9fa206ae158239be1142f748338be0840a0fb1258b71de33d031ee90ee59b;
    bytes32 internal constant DOMAIN_SEPARATOR = 0x7b6062683cdf5554ed30e3116f4e43854a6286baade707e86541123091f6eb81;
    bytes32 internal constant OFFER_HASH = 0x8dbd5dcf642988d717b308cc79507ae5a6bdfc8126f74b9f8182001bababeeb8;

    function test_offerHashVector() public {
        // Same bytecode at a fixed address and chain, so the domain separator is reproducible.
        vm.chainId(421_614);
        vm.etch(AT, address(escrow).code);
        Escrow e = Escrow(AT);
        Escrow.Offer memory o = Escrow.Offer({
            seller: 0x1111111111111111111111111111111111111111,
            buyer: 0x2222222222222222222222222222222222222222,
            token: 0x3333333333333333333333333333333333333333,
            amount: 45_000_000,
            feePayer: FeeMath.Payer.Half,
            maxFeeBps: 150,
            maxBaseGas: 100_000,
            maxMinFee: 1_000_000,
            bond: 10_000_000,
            lateSlashBps: 2500,
            instant: false,
            shipWindow: 259_200,
            confirmWindow: 604_800,
            listingHash: 0xabababababababababababababababababababababababababababababababab,
            expiry: 1_800_000_000,
            maxFills: 1,
            nonce: type(uint256).max
        });
        (, string memory name, string memory version, uint256 chainId, address verifyingContract,,) = e.eip712Domain();
        bytes32 separator = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256(bytes(name)),
                keccak256(bytes(version)),
                chainId,
                verifyingContract
            )
        );
        bytes32 h = e.hashOffer(o);
        console2.log("typehash");
        console2.logBytes32(e.OFFER_TYPEHASH());
        console2.log("domainSeparator");
        console2.logBytes32(separator);
        console2.log("offerHash");
        console2.logBytes32(h);
        assertEq(verifyingContract, AT);
        assertEq(h, keccak256(abi.encodePacked("\x19\x01", separator, keccak256(abi.encode(e.OFFER_TYPEHASH(), o)))));
        assertEq(e.OFFER_TYPEHASH(), TYPEHASH);
        assertEq(separator, DOMAIN_SEPARATOR);
        assertEq(h, OFFER_HASH);
    }
}
