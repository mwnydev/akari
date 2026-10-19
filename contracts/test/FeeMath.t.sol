// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Test} from "forge-std/Test.sol";
import {stdJson} from "forge-std/StdJson.sol";
import {FeeMath} from "../src/libraries/FeeMath.sol";

/// @dev External wrapper so reverts can be asserted with vm.expectRevert.
contract FeeMathHarness {
    function streamUnlocked(uint256 net, uint256 elapsed, uint256 periods) external pure returns (uint256) {
        return FeeMath.streamUnlocked(net, elapsed, periods);
    }
}

contract FeeMathTest is Test {
    using stdJson for string;

    string internal json;
    FeeMathHarness internal harness;

    function setUp() public {
        json = vm.readFile(string.concat(vm.projectRoot(), "/../test-vectors/fees.json"));
        harness = new FeeMathHarness();
    }

    function _uint(string memory key) internal view returns (uint256) {
        return vm.parseUint(json.readString(key));
    }

    function _payer(string memory s) internal pure returns (FeeMath.Payer) {
        bytes32 h = keccak256(bytes(s));
        if (h == keccak256("seller")) return FeeMath.Payer.Seller;
        if (h == keccak256("buyer")) return FeeMath.Payer.Buyer;
        if (h == keccak256("half")) return FeeMath.Payer.Half;
        revert("unknown payer");
    }

    function test_vectorVersion() public view {
        assertEq(json.readUint(".version"), 1);
    }

    function test_feeVectors() public view {
        uint256 n;
        for (; vm.keyExistsJson(json, string.concat(".fees[", vm.toString(n), "]")); n++) {
            string memory p = string.concat(".fees[", vm.toString(n), "]");
            string memory name = json.readString(string.concat(p, ".name"));
            uint256 amount = _uint(string.concat(p, ".amount"));
            uint256 minFee = _uint(string.concat(p, ".minFee"));
            FeeMath.Payer payer = _payer(json.readString(string.concat(p, ".feePayer")));

            uint256 total = FeeMath.feeTotal(
                amount, json.readUint(string.concat(p, ".feeBps")), minFee, json.readBool(string.concat(p, ".campaign"))
            );
            (uint256 bf, uint256 sf) = FeeMath.split(total, payer);
            (uint256 bg, uint256 sg) = FeeMath.split(_uint(string.concat(p, ".baseGas")), payer);

            assertEq(bf, _uint(string.concat(p, ".expect.buyerFee")), name);
            assertEq(sf, _uint(string.concat(p, ".expect.sellerFee")), name);
            assertEq(bg, _uint(string.concat(p, ".expect.buyerGas")), name);
            assertEq(sg, _uint(string.concat(p, ".expect.sellerGas")), name);
            assertEq(FeeMath.buyerPays(amount, bf, bg), _uint(string.concat(p, ".expect.buyerPays")), name);
            assertEq(
                FeeMath.disputeFee(total, minFee, json.readUint(string.concat(p, ".disputeMulBps"))),
                _uint(string.concat(p, ".expect.disputeFee")),
                name
            );
        }
        assertGt(n, 0, "no fee vectors");
    }

    function test_streamVectors() public view {
        uint256 n;
        for (; vm.keyExistsJson(json, string.concat(".streams[", vm.toString(n), "]")); n++) {
            string memory p = string.concat(".streams[", vm.toString(n), "]");
            string memory name = json.readString(string.concat(p, ".name"));
            uint256 amount = _uint(string.concat(p, ".amount"));
            FeeMath.Payer payer = _payer(json.readString(string.concat(p, ".feePayer")));

            uint256 total = FeeMath.feeTotal(
                amount, json.readUint(string.concat(p, ".feeBps")), _uint(string.concat(p, ".minFee")), false
            );
            (, uint256 sf) = FeeMath.split(total, payer);
            (, uint256 sg) = FeeMath.split(_uint(string.concat(p, ".baseGas")), payer);
            uint256 net = FeeMath.streamNet(amount, sf, sg);
            assertEq(net, _uint(string.concat(p, ".expect.unlockableNet")), name);

            uint256 periods = json.readUint(string.concat(p, ".periods"));
            uint256 k;
            for (; vm.keyExistsJson(json, string.concat(p, ".expect.points[", vm.toString(k), "]")); k++) {
                string memory q = string.concat(p, ".expect.points[", vm.toString(k), "]");
                assertEq(
                    FeeMath.streamUnlocked(net, json.readUint(string.concat(q, ".elapsedPeriods")), periods),
                    _uint(string.concat(q, ".unlocked")),
                    name
                );
            }
            assertGt(k, 0, "no stream points");
        }
        assertGt(n, 0, "no stream vectors");
    }

    function test_cnyToUsdtVectors() public view {
        uint256 n;
        for (; vm.keyExistsJson(json, string.concat(".cny.toUsdt[", vm.toString(n), "]")); n++) {
            string memory p = string.concat(".cny.toUsdt[", vm.toString(n), "]");
            assertEq(
                FeeMath.cnyFenToUsdt(
                    _uint(string.concat(p, ".priceFen")), json.readUint(string.concat(p, ".cnyPerUsdt"))
                ),
                _uint(string.concat(p, ".expect")),
                json.readString(string.concat(p, ".name"))
            );
        }
        assertGt(n, 0, "no cny vectors");
    }

    function test_claimNet() public view {
        uint256 claimGas = _uint(".claimGas");
        assertEq(FeeMath.claimNet(1_000_000, 0, claimGas), 980_000);
        assertEq(FeeMath.claimNet(20_000, 0, claimGas), 0, "claimable == claimGas not allowed");
        assertEq(FeeMath.claimNet(20_001, 0, claimGas), 1);
        assertEq(FeeMath.claimNet(500, 500, claimGas), 0);
    }

    function test_streamUnlocked_zeroPeriodsReverts() public {
        vm.expectRevert(FeeMath.ZeroPeriods.selector);
        harness.streamUnlocked(1, 0, 0);
    }

    // ---- fuzz ----

    function testFuzz_splitSumsToTotal(uint256 total, uint8 p) public pure {
        (uint256 b, uint256 s) = FeeMath.split(total, FeeMath.Payer(p % 3));
        assertEq(b + s, total);
        if (p % 3 == 2) assertLe(s - b, 1);
    }

    function testFuzz_feeTotalBounds(uint128 amount, uint16 bps, uint64 minFee, bool campaign) public pure {
        bps = uint16(bound(bps, 0, 1000));
        uint256 fee = FeeMath.feeTotal(amount, bps, minFee, campaign);
        uint256 raw = uint256(amount) * bps / 10_000;
        if (campaign) assertEq(fee, raw);
        else assertEq(fee, raw > minFee ? raw : minFee);
    }

    function testFuzz_streamUnlockedMonotonic(uint128 net, uint16 periods, uint16 e1, uint16 e2) public pure {
        periods = uint16(bound(periods, 1, 8760)); // hourly x 365 days
        (uint256 lo, uint256 hi) = e1 <= e2 ? (e1, e2) : (e2, e1);
        uint256 a = FeeMath.streamUnlocked(net, lo, periods);
        uint256 b = FeeMath.streamUnlocked(net, hi, periods);
        assertLe(a, b);
        assertLe(b, net);
        assertEq(FeeMath.streamUnlocked(net, periods, periods), net);
        assertEq(FeeMath.streamUnlocked(net, 0, periods), 0);
    }

    /// The seller never receives less than the signed rate implies, and the buyer overpays by less than one unit.
    function testFuzz_cnyFenToUsdtRoundsUp(uint128 priceFen, uint32 cnyPerUsdt) public pure {
        cnyPerUsdt = uint32(bound(cnyPerUsdt, 50_000, 100_000));
        uint256 usdt = FeeMath.cnyFenToUsdt(priceFen, cnyPerUsdt);
        uint256 target = uint256(priceFen) * 1e8;
        assertGe(usdt * cnyPerUsdt, target);
        if (usdt > 0) assertLt((usdt - 1) * cnyPerUsdt, target);
    }

    function testFuzz_claimNetNeverExceedsClaimable(uint128 unlocked, uint128 claimed, uint32 claimGas) public pure {
        vm.assume(claimed <= unlocked);
        uint256 got = FeeMath.claimNet(unlocked, claimed, claimGas);
        assertLe(got, uint256(unlocked) - claimed);
        if (got > 0) assertEq(got + claimGas, uint256(unlocked) - claimed);
    }

    /// Fees are earned pro rata to the seller share; a retained dispute fee replaces them (charged = max(0, earned -
    /// retained)), and neither part ever exceeds its uncapped pro-rata value.
    /// Inputs are uint64: real amounts are capped at 100000 USDT (1e11 units).
    function testFuzz_feesCharged(uint64 buyerFee, uint64 sellerFee, uint64 base, uint64 s, uint64 retained)
        public
        pure
    {
        base = uint64(bound(base, 1, type(uint64).max));
        s = uint64(bound(s, 0, base));
        uint256 rawB = uint256(buyerFee) * s / base;
        uint256 rawS = uint256(sellerFee) * s / base;
        (uint256 feeB, uint256 feeS) = FeeMath.feesCharged(buyerFee, sellerFee, s, base, retained);
        assertLe(feeB, rawB);
        assertLe(feeS, rawS);
        uint256 raw = rawB + rawS;
        assertEq(feeB + feeS, raw > retained ? raw - retained : 0);
        if (retained == 0) {
            assertEq(feeB, rawB);
            assertEq(feeS, rawS);
        }
    }

    function test_feesCharged_examples() public pure {
        (uint256 b, uint256 s) = FeeMath.feesCharged(0, 1_000_000, 54_395_000, 98_900_000, 500_000);
        assertEq(b, 0);
        assertEq(s, 50_000); // 0.55 USDT earned - 0.5 retained
        (b, s) = FeeMath.feesCharged(600_000, 400_000, 1, 2, 0);
        assertEq(b, 300_000);
        assertEq(s, 200_000);
        (b, s) = FeeMath.feesCharged(600_000, 400_000, 2, 2, 2_000_000);
        assertEq(b + s, 0);
    }
}
