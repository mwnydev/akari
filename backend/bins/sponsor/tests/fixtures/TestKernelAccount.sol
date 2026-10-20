// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

// 集成测试用（backend/bins/sponsor/tests/relay.rs、backend/bins/api/tests/auction_e2e.rs），不部署到任何真实网络。
// 用 ECDSA 所有者代替 Passkey，但 callData 入口与 Kernel v3.3 相同：ERC-7579 execute(bytes32 mode, bytes)。

// 只为让 forge 一并编译 EntryPoint v0.7：测试从产物里取它的字节码。
import {EntryPoint} from "account-abstraction/core/EntryPoint.sol";
import {IAccount} from "account-abstraction/interfaces/IAccount.sol";
import {PackedUserOperation} from "account-abstraction/interfaces/PackedUserOperation.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

contract TestKernelAccount is IAccount {
    struct Execution {
        address target;
        uint256 value;
        bytes callData;
    }

    address public immutable entryPoint;
    address public immutable owner;

    /// 与生产钱包的 initConfig 一样，部署时就给 Paymaster 授权 USDT（ADR 0004、0008）。
    constructor(address entryPoint_, address owner_, IERC20 token, address paymaster) {
        entryPoint = entryPoint_;
        owner = owner_;
        token.approve(paymaster, type(uint256).max);
    }

    function validateUserOp(PackedUserOperation calldata op, bytes32 hash, uint256 missing)
        external
        returns (uint256)
    {
        require(msg.sender == entryPoint, "not entry point");
        if (missing > 0) {
            (bool ok,) = payable(msg.sender).call{value: missing}("");
            ok;
        }
        (address r,,) = ECDSA.tryRecover(MessageHashUtils.toEthSignedMessageHash(hash), op.signature);
        return r == owner ? 0 : 1;
    }

    /// ERC-1271：所有者对摘要的 ECDSA 签名（拍卖授权用，`backend/bins/api/tests/auction_e2e.rs`）。
    function isValidSignature(bytes32 hash, bytes calldata sig) external view returns (bytes4) {
        (address r,,) = ECDSA.tryRecover(hash, sig);
        return r == owner ? bytes4(0x1626ba7e) : bytes4(0xffffffff);
    }

    function execute(bytes32 mode, bytes calldata data) external payable {
        require(msg.sender == entryPoint || msg.sender == owner, "not allowed");
        bytes1 callType = mode[0];
        if (callType == 0x00) {
            _call(address(bytes20(data[:20])), uint256(bytes32(data[20:52])), data[52:]);
        } else if (callType == 0x01) {
            Execution[] memory ex = abi.decode(data, (Execution[]));
            for (uint256 i; i < ex.length; ++i) {
                _call(ex[i].target, ex[i].value, ex[i].callData);
            }
        } else {
            revert("unsupported call type");
        }
    }

    function _call(address target, uint256 value, bytes memory data) internal {
        (bool ok, bytes memory ret) = target.call{value: value}(data);
        if (!ok) {
            assembly {
                revert(add(ret, 32), mload(ret))
            }
        }
    }
}

contract TestKernelFactory {
    function createAccount(address entryPoint, address owner, IERC20 token, address paymaster, bytes32 salt)
        external
        returns (address a)
    {
        a = getAddress(entryPoint, owner, token, paymaster, salt);
        if (a.code.length == 0) new TestKernelAccount{salt: salt}(entryPoint, owner, token, paymaster);
    }

    function getAddress(address entryPoint, address owner, IERC20 token, address paymaster, bytes32 salt)
        public
        view
        returns (address)
    {
        bytes32 h = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                address(this),
                salt,
                keccak256(
                    abi.encodePacked(
                        type(TestKernelAccount).creationCode, abi.encode(entryPoint, owner, token, paymaster)
                    )
                )
            )
        );
        return address(uint160(uint256(h)));
    }
}
