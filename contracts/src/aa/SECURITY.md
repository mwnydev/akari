# FixedFeePaymaster 安全说明

不做付费审计（站长决定），本文件给自审 / 社区审查者用：写清合约信任什么、不信任什么、应该重点查哪里。设计依据见 `docs/adr/0004-paymaster-minimal-fixed-fee.md`。

## 合约做什么

- EntryPoint **v0.7**（`0x0000000071727De22E5E9d8BAf0edAc6f37da032`）上的 Paymaster，用自己在 EntryPoint 的 ETH 押金替用户付 gas。
- 每个 UserOperation 带一份 forum-sponsor 的签名，签名里写定固定费 `fee`（USDT 最小单位）。`fee > 0` 时在**验证阶段**从用户账户 `transferFrom` 到 Paymaster 自己；不实现 `postOp`。
- 费用累积在 Paymaster，任何人调用 `sweepFees()` 转到构造时写死的 `treasury`（金库 Safe）。
- 不可升级，无 `selfdestruct`，无任何从用户账户拉钱的其他路径。

## 信任假设

| 角色 / 组件 | 被信任做什么 | 被攻破时最坏结果 |
| --- | --- | --- |
| `signer`（forum-sponsor 私钥，服务器 systemd-creds） | 只给本站用户的白名单操作签名，签名有效期 2 分钟（`validAfter` 填当前时间） | 攻击者可以签任意操作，耗尽 Paymaster 的 ETH 押金（约 2 周用量）；**不能**拿走用户的 USDT：每笔操作仍需用户自己的 Passkey 签名，且单笔费用 ≤ 1 USDT |
| `owner`（运营 Safe） | 轮换 `signer`；提取 / 管理 ETH 押金与质押 | 取走 ETH 押金；**不能**动用户 USDT 或已收的费用（费用只能去 `treasury`） |
| `treasury`（金库 Safe，不可变） | 收费 | — |
| `token`（USD₮0，不可变） | 标准 ERC-20 行为 | USD₮0 是可升级代理，发行方可冻结地址；冻结 Paymaster 地址会让所有收费操作验证失败（应急：部署新 Paymaster 并在前端 / sponsor 切换） |
| EntryPoint v0.7 | 账户签名失败时整笔 `handleOps` 回滚（AA24）；不调用空 context 的 `postOp` | 依赖 eth-infinitism 已审计的 v0.7 实现 |
| 用户账户（ZeroDev Kernel v3.3） | 部署时 `initCode` 给 Paymaster 授权 USDT 额度 | 授权额度只能经本合约的验证路径使用，且每笔需 sponsor 签名 + 用户签名 |

## 审查重点

1. **费用不会被绕过**：`getHash` 覆盖 sender、nonce、initCode、callData、全部 gas 字段、chainid、本合约地址、`validUntil`、`validAfter`、`fee`。用户改 `fee` 或 callData → 签名失效（测试 `test_rejects_tamperedFee`、`test_rejects_tamperedCallData`）。
2. **业务失败照样收费**：费用在验证阶段转走，callData 回滚不影响（`test_feeCollectedWhenCallDataReverts`、`testFuzz_chargesExactlySignedFee`）；callData 也无法先把余额转走（`test_callDataCannotDrainBeforeFee`）。
3. **签名失败不收费**：签名不对返回 `SIG_VALIDATION_FAILED`，不执行转账（`test_rejects_forgedSignature_noFeeTaken`）。
4. **`transferFrom` 的 from 是任意 sender**（Slither `arbitrary-send-erc20`，已注释抑制）：sender 是该 UserOperation 自己的账户；若账户签名无效，EntryPoint 在 Paymaster 验证之后整笔回滚，转账随之撤销。已核对 EntryPoint v0.7 `_validateAccountAndPaymasterValidationData`（先验证账户、再验证 Paymaster、最后才检查账户签名并 revert），测试 `test_rejects_badAccountSignature_feeTransferUndone` 覆盖。
5. **费用上限**：`MAX_FEE = 1 USDT` 是常量，超过直接 revert（`test_rejects_feeAboveCap`）。forum-sponsor 与钱包确认页另有同样的硬上限。
6. **时间窗口在链上强制**：`validUntil == 0`（EntryPoint 里表示永不过期）、`validUntil < validAfter`、或 `validUntil - validAfter > MAX_VALIDITY`（10 分钟）都按签名失败处理，不收费。验证阶段不能读 `block.timestamp`（ERC-7562），所以用窗口长度约束：forum-sponsor 必须把 `validAfter` 设为当前时间（测试 `test_rejects_noExpiry_noFeeTaken`、`test_rejects_windowTooLong`、`test_rejects_windowWithoutValidAfter`、`test_rejects_invertedWindow`、`test_accepts_maxWindow`）。
7. **重放**：靠账户 nonce（EntryPoint 管理）+ chainid + Paymaster 地址进哈希（`test_rejects_replayedOp`、`test_hashBindsChainAndPaymaster`）。

## ERC-7562 存储规则（只影响备用的公共 Bundler）

- 私有代发器不执行 ERC-7562，主路径不受影响。
- 公共 Bundler 要求 Paymaster **质押**（`addStake`）。验证阶段写的是 `balance[sender]`、`allowance[sender][paymaster]`、`balance[paymaster]`，对已质押实体都允许；**不能**直接写 `balance[treasury]`，所以费用先留在 Paymaster 再 sweep。
- USD₮0 是代理合约，验证时会读实现地址槽，不属于 sender / paymaster 的关联存储。**切换到公共 Bundler 前必须在测试网实测是否被拒**；被拒时备用路径只能用于不收费（`fee = 0`）的操作。

## 已知限制

- 押金耗尽时所有代付操作失败：押金只放约 2 周用量，余额告警（plan 第 9 节）。
- 本合约不做每账户限频、不做白名单方法检查——这些都在 forum-sponsor 签名前完成。合约只保证「签了什么就收什么，最多 1 USDT」。
