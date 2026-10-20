// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {AATestBase} from "./AATestBase.sol";
import {FixedFeePaymaster} from "../../src/aa/FixedFeePaymaster.sol";
import {MockUSDT} from "../../src/MockUSDT.sol";
import {IEntryPoint} from "account-abstraction/interfaces/IEntryPoint.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {SimpleAccount} from "account-abstraction/samples/SimpleAccount.sol";
import {SimpleAccountFactory} from "account-abstraction/samples/SimpleAccountFactory.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Vm} from "forge-std/Vm.sol";

contract FixedFeePaymasterTest is AATestBase {
    FixedFeePaymaster internal pm;
    MockUSDT internal usdt;
    SimpleAccount internal account;

    address internal treasury = makeAddr("treasury");
    address internal opsSafe = makeAddr("opsSafe");
    address internal recipient = makeAddr("recipient");
    uint256 internal ownerKey;
    uint256 internal signerKey;
    address internal signerAddr;

    uint128 internal constant FEE = 50_000; // 0.05 USDT
    uint256 internal constant START_BALANCE = 10_000_000; // 10 USDT

    function setUp() public {
        vm.warp(1_800_000_000);
        _deployEntryPoint();
        usdt = new MockUSDT();
        (signerAddr, signerKey) = makeAddrAndKey("sponsor");
        address owner;
        (owner, ownerKey) = makeAddrAndKey("accountOwner");

        pm = new FixedFeePaymaster(
            IEntryPoint(address(entryPoint)), IERC20(address(usdt)), treasury, signerAddr, opsSafe
        );
        pm.deposit{value: 10 ether}();

        SimpleAccountFactory factory = new SimpleAccountFactory(IEntryPoint(address(entryPoint)));
        account = factory.createAccount(owner, 0);
        usdt.mint(address(account), START_BALANCE);
        // In production this allowance is granted by the account's initCode at deployment (docs/adr/0004).
        vm.prank(owner);
        account.execute(address(usdt), 0, abi.encodeCall(IERC20.approve, (address(pm), type(uint256).max)));
    }

    // ------------------------------------------------------------------ helpers

    function _transferCall(uint256 amount) internal view returns (bytes memory) {
        return
            abi.encodeCall(
                SimpleAccount.execute, (address(usdt), 0, abi.encodeCall(IERC20.transfer, (recipient, amount)))
            );
    }

    function _pmPrefix(uint48 validUntil, uint48 validAfter, uint128 fee) internal view returns (bytes memory) {
        return abi.encodePacked(address(pm), PM_VERIFICATION_GAS, uint128(0), validUntil, validAfter, fee);
    }

    /// @dev Builds a fully signed op. `signFee` lets a test sign a different fee from the one in paymasterAndData.
    function _op(bytes memory callData, uint48 validUntil, uint48 validAfter, uint128 fee, uint128 signFee, uint256 key)
        internal
        view
        returns (PackedUserOperation memory op)
    {
        op = _baseOp(address(account), callData);
        op.paymasterAndData = _pmPrefix(validUntil, validAfter, fee);
        bytes32 h = pm.getHash(op, validUntil, validAfter, signFee);
        op.paymasterAndData = abi.encodePacked(op.paymasterAndData, _signEthPrefixed(key, h));
        op.signature = _signEthPrefixed(ownerKey, entryPoint.getUserOpHash(op));
    }

    function _validOp(bytes memory callData, uint128 fee) internal view returns (PackedUserOperation memory) {
        return _op(callData, uint48(block.timestamp + 120), uint48(block.timestamp), fee, fee, signerKey);
    }

    function _expectFailedOp(PackedUserOperation memory op, string memory reason) internal {
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOp.selector, 0, reason));
        vm.prank(bundler, bundler);
        entryPoint.handleOps(ops, bundler);
    }

    function _expectFailedOpWithRevert(PackedUserOperation memory op, bytes memory inner) internal {
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.expectRevert(abi.encodeWithSelector(IEntryPoint.FailedOpWithRevert.selector, 0, "AA33 reverted", inner));
        vm.prank(bundler, bundler);
        entryPoint.handleOps(ops, bundler);
    }

    // ------------------------------------------------------------------ constructor

    function test_constructor_setsState() public view {
        assertEq(address(pm.token()), address(usdt));
        assertEq(pm.treasury(), treasury);
        assertEq(pm.signer(), signerAddr);
        assertEq(pm.owner(), opsSafe);
        assertEq(address(pm.entryPoint()), address(entryPoint));
        assertEq(pm.MAX_FEE(), 1_000_000);
    }

    function test_constructor_rejectsZeroAddresses() public {
        IEntryPoint ep = IEntryPoint(address(entryPoint));
        IERC20 t = IERC20(address(usdt));
        vm.expectRevert(FixedFeePaymaster.ZeroAddress.selector);
        new FixedFeePaymaster(ep, IERC20(address(0)), treasury, signerAddr, opsSafe);
        vm.expectRevert(FixedFeePaymaster.ZeroAddress.selector);
        new FixedFeePaymaster(ep, t, address(0), signerAddr, opsSafe);
        vm.expectRevert(FixedFeePaymaster.ZeroAddress.selector);
        new FixedFeePaymaster(ep, t, treasury, address(0), opsSafe);
        vm.expectRevert(FixedFeePaymaster.ZeroAddress.selector);
        new FixedFeePaymaster(ep, t, treasury, signerAddr, address(0));
    }

    // ------------------------------------------------------------------ fee collection

    function test_success_chargesFixedFee() public {
        vm.expectEmit(address(pm));
        emit FixedFeePaymaster.FeeCharged(address(account), FEE);
        _handle(_validOp(_transferCall(1_000_000), FEE));
        assertEq(usdt.balanceOf(address(pm)), FEE);
        assertEq(usdt.balanceOf(recipient), 1_000_000);
        assertEq(usdt.balanceOf(address(account)), START_BALANCE - 1_000_000 - FEE);
    }

    function test_feeCollectedWhenCallDataReverts() public {
        // Transfer more than the account holds: callData reverts inside the EntryPoint, the op is still included.
        uint256 depositBefore = entryPoint.balanceOf(address(pm));
        _handle(_validOp(_transferCall(START_BALANCE * 2), FEE));
        assertEq(usdt.balanceOf(address(pm)), FEE, "fee kept although callData reverted");
        assertEq(usdt.balanceOf(recipient), 0);
        assertLt(entryPoint.balanceOf(address(pm)), depositBefore, "paymaster paid the gas");
    }

    function test_callDataCannotDrainBeforeFee() public {
        // The op tries to move the whole starting balance; the fee was already taken during validation,
        // so the transfer fails and the paymaster still holds the fee.
        _handle(_validOp(_transferCall(START_BALANCE), FEE));
        assertEq(usdt.balanceOf(address(pm)), FEE);
        assertEq(usdt.balanceOf(recipient), 0);
    }

    /// A zero fee does not touch the token at all (no storage access that a public bundler could reject, ERC-7562).
    function test_zeroFee_noTransferNeeded() public {
        address owner = vm.addr(ownerKey);
        vm.prank(owner);
        account.execute(address(usdt), 0, abi.encodeCall(IERC20.approve, (address(pm), 0)));
        vm.recordLogs();
        _handle(_validOp(_transferCall(1_000_000), 0));
        Vm.Log[] memory logs = vm.getRecordedLogs();
        for (uint256 i; i < logs.length; ++i) {
            assertTrue(logs[i].topics[0] != FixedFeePaymaster.FeeCharged.selector, "no FeeCharged for a zero fee");
        }
        assertEq(usdt.balanceOf(address(pm)), 0);
        assertEq(usdt.balanceOf(recipient), 1_000_000);
    }

    function test_maxFee_allowed() public {
        _handle(_validOp(_transferCall(1), 1_000_000));
        assertEq(usdt.balanceOf(address(pm)), 1_000_000);
    }

    function testFuzz_chargesExactlySignedFee(uint128 fee, uint256 amount) public {
        fee = uint128(bound(fee, 0, 1_000_000));
        amount = bound(amount, 0, START_BALANCE * 2);
        _handle(_validOp(_transferCall(amount), fee));
        assertEq(usdt.balanceOf(address(pm)), fee, "fee charged regardless of callData outcome");
        assertEq(usdt.balanceOf(address(account)) + usdt.balanceOf(recipient) + fee, START_BALANCE, "no value lost");
    }

    // ------------------------------------------------------------------ validation failures

    function test_rejects_feeAboveCap() public {
        PackedUserOperation memory op = _validOp(_transferCall(1), 1_000_001);
        _expectFailedOpWithRevert(op, abi.encodeWithSelector(FixedFeePaymaster.FeeAboveCap.selector, 1_000_001));
    }

    function test_rejects_insufficientAllowance() public {
        address owner = vm.addr(ownerKey);
        vm.prank(owner);
        account.execute(address(usdt), 0, abi.encodeCall(IERC20.approve, (address(pm), FEE - 1)));
        PackedUserOperation memory op = _validOp(_transferCall(1), FEE);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.expectPartialRevert(IEntryPoint.FailedOpWithRevert.selector);
        vm.prank(bundler, bundler);
        entryPoint.handleOps(ops, bundler);
        assertEq(usdt.balanceOf(address(pm)), 0);
    }

    function test_rejects_insufficientBalance() public {
        vm.prank(address(account));
        usdt.transfer(recipient, START_BALANCE - FEE + 1);
        PackedUserOperation memory op = _validOp(_transferCall(1), FEE);
        PackedUserOperation[] memory ops = new PackedUserOperation[](1);
        ops[0] = op;
        vm.expectPartialRevert(IEntryPoint.FailedOpWithRevert.selector);
        vm.prank(bundler, bundler);
        entryPoint.handleOps(ops, bundler);
    }

    function test_rejects_forgedSignature_noFeeTaken() public {
        (, uint256 attackerKey) = makeAddrAndKey("attacker");
        PackedUserOperation memory op =
            _op(_transferCall(1), uint48(block.timestamp + 120), uint48(block.timestamp), FEE, FEE, attackerKey);
        _expectFailedOp(op, "AA34 signature error");
        assertEq(usdt.balanceOf(address(pm)), 0);
    }

    function test_rejects_badAccountSignature_feeTransferUndone() public {
        // Valid sponsor signature but the account owner did not sign: the paymaster pulls the fee during validation,
        // then the EntryPoint rejects the account signature (AA24) and the whole handleOps reverts.
        PackedUserOperation memory op = _validOp(_transferCall(1), FEE);
        (, uint256 attackerKey) = makeAddrAndKey("attacker");
        op.signature = _signEthPrefixed(attackerKey, entryPoint.getUserOpHash(op));
        _expectFailedOp(op, "AA24 signature error");
        assertEq(usdt.balanceOf(address(pm)), 0);
        assertEq(usdt.balanceOf(address(account)), START_BALANCE);
    }

    function test_rejects_tamperedFee() public {
        // Sponsor signed fee = FEE, but paymasterAndData claims 0 (user trying to skip the fee).
        PackedUserOperation memory op =
            _op(_transferCall(1), uint48(block.timestamp + 120), uint48(block.timestamp), 0, FEE, signerKey);
        _expectFailedOp(op, "AA34 signature error");
    }

    function test_rejects_tamperedCallData() public {
        PackedUserOperation memory op = _validOp(_transferCall(1), FEE);
        op.callData = _transferCall(2);
        op.signature = _signEthPrefixed(ownerKey, entryPoint.getUserOpHash(op));
        _expectFailedOp(op, "AA34 signature error");
    }

    function test_rejects_malformedSignature() public {
        PackedUserOperation memory op = _baseOp(address(account), _transferCall(1));
        op.paymasterAndData = abi.encodePacked(_pmPrefix(uint48(block.timestamp + 120), 0, FEE), hex"1234");
        op.signature = _signEthPrefixed(ownerKey, entryPoint.getUserOpHash(op));
        _expectFailedOp(op, "AA34 signature error");
    }

    function test_rejects_expired() public {
        PackedUserOperation memory op =
            _op(_transferCall(1), uint48(block.timestamp - 1), uint48(block.timestamp - 121), FEE, FEE, signerKey);
        _expectFailedOp(op, "AA32 paymaster expired or not due");
    }

    function test_rejects_notYetValid() public {
        PackedUserOperation memory op =
            _op(_transferCall(1), uint48(block.timestamp + 600), uint48(block.timestamp + 300), FEE, FEE, signerKey);
        _expectFailedOp(op, "AA32 paymaster expired or not due");
    }

    function test_rejects_noExpiry_noFeeTaken() public {
        PackedUserOperation memory op = _op(_transferCall(1), 0, 0, FEE, FEE, signerKey);
        _expectFailedOp(op, "AA34 signature error");
        // Also with a validAfter set: still a signature failure, never a revert.
        op = _op(_transferCall(1), 0, uint48(block.timestamp), FEE, FEE, signerKey);
        _expectFailedOp(op, "AA34 signature error");
        assertEq(usdt.balanceOf(address(pm)), 0);
    }

    function test_rejects_windowTooLong() public {
        uint48 start = uint48(block.timestamp);
        PackedUserOperation memory op = _op(_transferCall(1), start + 10 minutes + 1, start, FEE, FEE, signerKey);
        _expectFailedOp(op, "AA34 signature error");
        assertEq(usdt.balanceOf(address(pm)), 0);
    }

    function test_rejects_windowWithoutValidAfter() public {
        // validAfter = 0 makes the window span all of history, so it is over-long by definition.
        PackedUserOperation memory op = _op(_transferCall(1), uint48(block.timestamp + 120), 0, FEE, FEE, signerKey);
        _expectFailedOp(op, "AA34 signature error");
    }

    function test_rejects_invertedWindow() public {
        uint48 start = uint48(block.timestamp);
        PackedUserOperation memory op = _op(_transferCall(1), start, start + 1, FEE, FEE, signerKey);
        _expectFailedOp(op, "AA34 signature error");
    }

    function test_accepts_maxWindow() public {
        uint48 start = uint48(block.timestamp);
        _handle(_op(_transferCall(1), start + 10 minutes, start, FEE, FEE, signerKey));
        assertEq(usdt.balanceOf(address(pm)), FEE);
    }

    /// Exactly the fixed fields with no signature bytes is a signature failure, not malformed data.
    function test_rejects_missingSignature() public {
        PackedUserOperation memory op = _baseOp(address(account), _transferCall(1));
        op.paymasterAndData = _pmPrefix(uint48(block.timestamp + 120), uint48(block.timestamp), FEE);
        op.signature = _signEthPrefixed(ownerKey, entryPoint.getUserOpHash(op));
        _expectFailedOp(op, "AA34 signature error");
    }

    function test_rejects_shortPaymasterData() public {
        PackedUserOperation memory op = _baseOp(address(account), _transferCall(1));
        op.paymasterAndData = abi.encodePacked(address(pm), PM_VERIFICATION_GAS, uint128(0), uint48(0));
        op.signature = _signEthPrefixed(ownerKey, entryPoint.getUserOpHash(op));
        _expectFailedOpWithRevert(op, abi.encodeWithSelector(FixedFeePaymaster.MalformedPaymasterData.selector));
    }

    function test_rejects_replayedOp() public {
        PackedUserOperation memory op = _validOp(_transferCall(1), FEE);
        _handle(op);
        _expectFailedOp(op, "AA25 invalid account nonce");
    }

    function test_hashBindsChainAndPaymaster() public {
        PackedUserOperation memory op = _validOp(_transferCall(1), FEE);
        bytes32 h = pm.getHash(op, 1, 0, FEE);
        vm.chainId(block.chainid + 1);
        assertTrue(pm.getHash(op, 1, 0, FEE) != h, "chain id bound");
        assertTrue(pm.getHash(op, 1, 0, FEE + 1) != h, "fee bound");
        assertTrue(pm.getHash(op, 2, 0, FEE) != h, "validUntil bound");
    }

    // ------------------------------------------------------------------ owner & sweep

    function test_setSigner_onlyOwner() public {
        address newSigner = makeAddr("newSigner");
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, address(this)));
        pm.setSigner(newSigner);

        vm.expectEmit(address(pm));
        emit FixedFeePaymaster.SignerChanged(signerAddr, newSigner);
        vm.prank(opsSafe);
        pm.setSigner(newSigner);
        assertEq(pm.signer(), newSigner);

        // Ops signed by the old key are now rejected.
        _expectFailedOp(_validOp(_transferCall(1), FEE), "AA34 signature error");
    }

    function test_setSigner_rejectsZero() public {
        vm.prank(opsSafe);
        vm.expectRevert(FixedFeePaymaster.ZeroAddress.selector);
        pm.setSigner(address(0));
    }

    function test_sweepFees_anyoneToTreasury() public {
        _handle(_validOp(_transferCall(1), FEE));
        vm.expectEmit(address(pm));
        emit FixedFeePaymaster.FeesSwept(FEE);
        vm.prank(makeAddr("anyone"));
        pm.sweepFees();
        assertEq(usdt.balanceOf(treasury), FEE);
        assertEq(usdt.balanceOf(address(pm)), 0);
    }

    function test_depositAndStake_onlyOwner() public {
        address payable to = payable(makeAddr("withdrawTo"));
        vm.expectRevert(abi.encodeWithSelector(Ownable.OwnableUnauthorizedAccount.selector, address(this)));
        pm.withdrawTo(to, 1 ether);
        vm.prank(opsSafe);
        pm.withdrawTo(to, 1 ether);
        assertEq(to.balance, 1 ether);

        vm.deal(opsSafe, 2 ether);
        vm.startPrank(opsSafe);
        pm.addStake{value: 1 ether}(1 days);
        pm.unlockStake();
        vm.warp(block.timestamp + 1 days);
        pm.withdrawStake(to);
        vm.stopPrank();
        assertEq(to.balance, 2 ether);
    }
}
