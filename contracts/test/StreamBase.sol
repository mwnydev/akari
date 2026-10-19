// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Escrow} from "../src/Escrow.sol";
import {EscrowConfig} from "../src/EscrowConfig.sol";
import {StreamEscrow} from "../src/StreamEscrow.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";
import {FreezableUSDT} from "./mocks/TestTokens.sol";
import {EscrowBase} from "./EscrowBase.sol";

/// @dev Same actors and defaults as EscrowBase, with StreamEscrow deployed and registered as the second market.
abstract contract StreamBase is EscrowBase {
    StreamEscrow internal stream;

    // Default stream: 100 USDT, daily x 10, seller pays → fee 1 USDT, base gas 0.1 → net 98.9 USDT.
    uint256 internal constant NET = 98_900_000;
    uint256 internal constant PER_DAY = 9_890_000;
    uint256 internal constant CLAIM_GAS = 20_000;
    uint256 internal constant DISPUTE_FEE = 1_000_000;

    function setUp() public virtual override {
        (seller, sellerKey) = makeAddrAndKey("seller");
        (platform, platformKey) = makeAddrAndKey("auctionSigner");
        usdt = new FreezableUSDT();
        (config, escrow, stream) = _deployBoth(address(usdt), defaultParams());
        _fund(buyer, 1_000_000 * U);
        _fund(seller, 1_000_000 * U);
    }

    /// Same pattern as script/Deploy.s.sol: both market addresses are predicted and listed at construction.
    function _deployBoth(address token, EscrowConfig.Params memory p)
        internal
        returns (EscrowConfig c, Escrow e, StreamEscrow s)
    {
        uint256 n = vm.getNonce(address(this));
        address[] memory markets = new address[](2);
        markets[0] = vm.computeCreateAddress(address(this), n + 1);
        markets[1] = vm.computeCreateAddress(address(this), n + 2);
        c = new EscrowConfig(owner, p, token, platform, markets);
        e = new Escrow(c, arbiter, feeRecipient);
        s = new StreamEscrow(e);
        assertEq(address(e), markets[0]);
        assertEq(address(s), markets[1]);
    }

    function _fund(address who, uint256 amt) internal override {
        super._fund(who, amt);
        vm.prank(who);
        usdt.approve(address(stream), type(uint256).max);
    }

    function _soffer(uint256 amount, FeeMath.Payer payer) internal view returns (StreamEscrow.Offer memory) {
        return StreamEscrow.Offer({
            seller: seller,
            buyer: address(0),
            token: address(usdt),
            amount: amount,
            feePayer: payer,
            maxFeeBps: 100,
            maxBaseGas: 100_000,
            maxClaimGas: 20_000,
            maxMinFee: 1 * U,
            bond: 0,
            lateSlashBps: 0,
            interval: StreamEscrow.Interval.Day,
            periods: 10,
            startWindow: 3 days,
            listingHash: keccak256("listing"),
            expiry: uint64(block.timestamp + 1 days),
            maxFills: 1,
            nonce: 1
        });
    }

    function _ssign(StreamEscrow.Offer memory o, uint256 key) internal view returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, stream.hashOffer(o));
        return abi.encodePacked(r, s, v);
    }

    function _screate(StreamEscrow.Offer memory o) internal returns (uint256 id) {
        bytes memory sig = _ssign(o, sellerKey);
        vm.prank(buyer);
        id = stream.createAndFund(o, sig);
    }

    function _screateDefault() internal returns (uint256) {
        return _screate(_soffer(100 * U, FeeMath.Payer.Seller));
    }

    function _start(uint256 id) internal {
        vm.prank(seller);
        stream.start(id);
    }

    function _claim(uint256 id) internal {
        vm.prank(seller);
        stream.claim(id);
    }

    function _order(uint256 id) internal view returns (StreamEscrow.Order memory) {
        return stream.getOrder(id);
    }
}
