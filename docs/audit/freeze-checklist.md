# 合约冻结清单

- 依据：ADR 0007（不做付费审计：链上限损 + 多方 AI 审计 + 符号执行 + 变异测试 + 4 周公开审查与小额赏金）、plan 第 6、9、14、16 节
- 冻结日：第 12 周（2027-01-04）；本清单生成于 2026-10-20，冻结前如有改动按第 11 节重新生成哈希
- 配套文件：[ai-audit-pack.md](ai-audit-pack.md)（多方 AI 审计提示词）、[bug-bounty.md](bug-bounty.md)（赏金草案）、`contracts/script/audit-bundle.sh`（审计用单文件包）

## 1. 范围与文件哈希

冻结范围：`Escrow`、`StreamEscrow`、`FixedFeePaymaster`，以及它们依赖的 `MarketBase`、`EscrowConfig`、`FeeMath`。`MockUSDT` 只上测试网，不在范围内（列出哈希便于核对测试网部署）。

| 文件 | 行数 | sha256 |
| --- | --- | --- |
| `contracts/src/Escrow.sol` | 641 | `6f792a3d301bde46fc873c4867e5fdee252d275dd4f99a0b069325c58b15fb98` |
| `contracts/src/StreamEscrow.sol` | 481 | `bbce4b12a1974e254f9950869254a9c2fefa196322c61ae009896cf3c3a40500` |
| `contracts/src/MarketBase.sol` | 74 | `778f29593ac4571b40aa510570672977b3c46c90b0600ab9a9bd6342d76887f7` |
| `contracts/src/EscrowConfig.sol` | 388 | `d3cb82f39d259275dc9774da7119e5f1e91b0516fb98ce00a20d19183dbd4b92` |
| `contracts/src/libraries/FeeMath.sol` | 95 | `344b472fcd47f9157ba067bcaae8ae6f99839cb96e993b1e84e95bb0d5cc38f0` |
| `contracts/src/aa/FixedFeePaymaster.sol` | 148 | `fa0c388c6b93dca0bb7212c42a656a0ac54a536d387c21b8f81ddd39edea5230` |
| `contracts/src/MockUSDT.sol`（仅测试网） | 18 | `4e08bd3c81841fc958ad2db77b831080328f5cc3ea954b4e479e561e3ecbbea1` |
| `contracts/foundry.toml`（编译设置） | — | `62846b14a92047bada21ff78170e30a4b9510b3771f3a4693ee88fd7315619ab` |

创建字节码（`out/<C>.sol/<C>.json` 的 `bytecode.object`，不含构造参数；末尾 CBOR 元数据含全部源文件哈希，所以任一依赖或编译设置变化都会改变它）：

| 合约 | keccak256(creation bytecode) | 运行时大小 |
| --- | --- | --- |
| Escrow | `0xcbc9cb27c1d6ecc2d9d9c684ee61bcf3b15d0d40458603935ef176ed3d83220c` | 22 509 B（余 2 067 B） |
| StreamEscrow | `0x951f74d9bc9f3ae51603cd15e7e397382cc0dd0d05f6142d40ffad0e0a50c524` | 16 929 B |
| EscrowConfig | `0xb924ebeafe27ff66a0f53436398c58d17eb9ad485eee9ce9965768dc78f964f6` | 11 317 B |
| FixedFeePaymaster | `0xa52113ced5f8862312244d1ff15ad97c1c114c482b4c2d35d2a9cbf631a9712c` | 5 133 B |

复现：

```sh
cd contracts
sha256sum src/Escrow.sol src/StreamEscrow.sol src/MarketBase.sol src/EscrowConfig.sol \
  src/libraries/FeeMath.sol src/aa/FixedFeePaymaster.sol src/MockUSDT.sol foundry.toml
forge build --sizes
for c in Escrow StreamEscrow EscrowConfig FixedFeePaymaster; do
  echo "$c $(cast keccak "$(jq -r .bytecode.object out/$c.sol/$c.json)")"; done
```

部署后在 Arbiscan 验证源码，并用 `cast code` 对照运行时字节码（不可变量位置除外）。

## 2. 编译与构建设置

| 项 | 值 |
| --- | --- |
| Foundry | 1.8.3（`cae51ad`，CI 用 `foundry-rs/foundry-toolchain@v1` 固定 `v1.8.3`） |
| solc | 0.8.37（所有 `src/` 文件 `pragma solidity 0.8.37` 精确版本） |
| 优化器 | 开，`runs = 200` |
| via-IR | 关 |
| EVM 版本 | `cancun`（Arbitrum One ArbOS 32+；`ReentrancyGuardTransient` 用 TSTORE / TLOAD） |
| 元数据 | `bytecodeHash = ipfs`（默认） |
| remappings | `@openzeppelin/contracts/`、`solady/`、`forge-std/`、`account-abstraction/` → `lib/` 下对应目录 |
| 代码大小 | 全部低于 24 576 B（`forge build --sizes`，Escrow 余量最小） |
| 测试编译 | `dynamic_test_linking = false`：开启时测试里的 `new` 变成 `vm.deployCode`，`vm.expectRevert(...); new X(...)` 会在第一个预期回滚处结束整个测试、跳过后面的检查（变异测试发现，第 7 节）。只影响测试，合约字节码不变 |

## 3. 依赖版本与审计状态

依赖全部以源码形式放在 `contracts/lib/`（不是子模块），升级经 PR 审查。

| 依赖 | 版本 | 用到的部分 | 审计状态 |
| --- | --- | --- | --- |
| OpenZeppelin Contracts | 5.6.1 | `SafeERC20`、`ReentrancyGuardTransient`、`EIP712`、`ECDSA`、`MessageHashUtils`、`Ownable` / `Ownable2Step`、`IERC20Metadata` | 自带审计表（`lib/openzeppelin-contracts/audits/README.md`）到 v5.5.0（2025-10，含 SafeERC20、ECDSA、SignatureChecker）；**5.6 的改动不在随包审计表内**，冻结前对照 OpenZeppelin 官网审计页确认 |
| Solady | 0.1.26 | `SignatureCheckerLib`（ERC-1271 / ERC-6492 验签）、`FixedPointMathLib.fullMulDivUp` | `lib/solady/audits/` 有 Cantina、Cantina/Spearbit（Coinbase）、Ackee 报告，**均不针对 0.1.26**；这两个模块的覆盖范围冻结前逐份核对。CNY 换算另有本项目的向量、模糊与符号检查（第 6 节 I9） |
| eth-infinitism account-abstraction | v0.7.0（只放了 `contracts/`） | `BasePaymaster`、`UserOperationLib`、`Helpers`、接口；测试用 `EntryPoint`、`SimpleAccount` | 2026-10-20 逐个比对 GitHub `v0.7.0` 标签：`BasePaymaster`、`UserOperationLib`、`Helpers`、`EntryPoint` 与全部用到的接口 sha256 一致。v0.7 有上游审计（eth-infinitism 仓库 audits 目录）；主网用已部署的 EntryPoint `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |
| forge-std | 1.17.0 | 只用于测试与脚本 | — |

`contracts/script/audit-bundle.sh` 的输出末尾列出了编译进合约的全部库文件。

## 4. 外部假设

| 组件 | 我们依赖的行为 | 不成立时的后果与对策 |
| --- | --- | --- |
| USD₮0（Arbitrum One `0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9`） | 6 位小数；标准 ERC-20，转账返回 bool 或不返回（`SafeERC20`）；没有转账手续费；不回调接收方 | 发行方可冻结地址：付给被冻结地址的款项记为 `credit`，本人用 `withdraw(token, to)` 转到别的地址（T-08）。**合约地址本身被冻结、代币暂停或代理升级改变语义**时所有出口都会失败，属发行方风险（第 8 节）。若开启转账手续费，`_pullExact` 让新订单回滚，已有订单照常结算 |
| EntryPoint v0.7（`0x0000000071727De22E5E9d8BAf0edAc6f37da032`） | 先验证账户、再验证 Paymaster，账户签名失败时整笔 `handleOps` 回滚（AA24）；按 `validUntil` / `validAfter` 检查时间窗口；`context` 为空时不调用 `postOp`；押金只有 owner 能取 | 已核对 v0.7 `_validateAccountAndPaymasterValidationData` 的顺序（`contracts/src/aa/SECURITY.md` 第 4 条），测试 `test_rejects_badAccountSignature_feeTransferUndone` 覆盖 |
| ZeroDev Kernel v3.3 + WebAuthn 验证器（ADR 0003） | 部署 `initCode` 给 Paymaster 预设 USDT 授权；对报价与拍卖授权给出 ERC-1271 签名；未部署时可用 ERC-6492 包装签名 | 验签走 Solady `isValidERC6492SignatureNow`；未部署账户需要 Solady 的 ERC-6492 验证器 `0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e` 已部署（`Deploy.s.sol` 在主网缺失时中止） |
| Safe（金库、仲裁、运营，2/3） | 按多签规则执行；能接收 ERC-20 | `Deploy.s.sol` 在 Arbitrum One 检查三者都是 2/3 Safe、仲裁 ≠ 金库、签名人不含部署者 |
| Arbitrum One | `block.timestamp` 单调、偏差远小于 24 小时；支持 Cancun 操作码 | 期限下限 24 小时；排序器停摆时仲裁 Safe 可 `extendDeadlines`（T-19） |
| 链下服务 | forum-sponsor 只为白名单操作签代付、签名有效期 2 分钟、`validAfter` = 当前时间；拍卖成交证明只给真实最高出价者 | 合约只保证「签了什么就收什么、最多 1 USDT、窗口 ≤ 10 分钟」与「每场拍卖只成交一次、不低于保留价」；私钥被盗的后果见第 8 节 |

## 5. 角色与权限

| 角色 | 持有人 | 能做 | 不能做 |
| --- | --- | --- | --- |
| `EscrowConfig.owner` | 金库 Safe（2/3，`Ownable2Step` 两步移交） | 在硬上限内改参数；暂停 / 恢复新订单；设免费活动；排队加代币、换拍卖签名人、加市场；立即停用代币；撤销排队项 | 动任何订单或保证金的钱；改已有订单（建单时快照）；超出硬上限；跳过 72 小时排队；移除市场 |
| `arbiter` | 仲裁 Safe（2/3），两个市场共用，构造后不可变 | `resolve`（只能按比例在买卖双方之间分配本金、保证金只能赔给该单买家）；`extendDeadlines`（只能延长未结订单期限，单次 ≤ 7 天、每单累计 ≤ 14 天） | 把钱付给第三方；缩短期限；动未争议订单的钱 |
| `feeRecipient` | 金库 Safe，不可变 | 只是收款地址（手续费、基础 gas 费、领取 gas 费、留存的争议费） | 任何调用权限 |
| 市场（`isMarket`） | Escrow、StreamEscrow（构造时登记；新增须排队 72 小时，不能移除） | `checkout`（取参数快照、占用 gas 补贴名额）、`lockFunds` / `unlockFunds`（在押计数）、Escrow 的 `lockBond` / `releaseBond` / `takeBond` | 释放多于自己锁定的保证金（下溢回滚）；把赔付给订单买家以外的人 |
| 买家 | 下单账户 | `createAndFund` / `createFromAuction`、`release`、紧急顺延确认期 1 次、提议 / 接受和解、发起争议 | — |
| 卖家 | 报价签名账户 | 签报价 / 拍卖授权、`cancelOffer`、`markDelivered` / `start`、`claim`、`refund`、延长确认期（累计 ≤ 30 天）、提议 / 接受和解、发起争议（钱包、本单已解锁余额或保证金付争议费）、存取保证金（只能取未锁定部分） | — |
| 任何人 | — | `claimTimeout`、`finalize`、`resolveTimeout`、`applyParams` / `applyCampaign` / `enableToken` / `applyAuctionSigner` / `addMarket`（排队到期后）、`sweepFees`、`withdraw`（只取自己的 credit） | — |
| `FixedFeePaymaster.owner` | 运营 Safe（`Ownable` 单步移交） | `setSigner`；`withdrawTo`、`addStake`、`unlockStake`、`withdrawStake`（平台自己的 ETH 押金） | 动用户 USDT；改费用上限；改 `treasury` / `token` |
| Paymaster `signer` | forum-sponsor 私钥 | 为 userOp 签代付（费用 ≤ 1 USDT、窗口 ≤ 10 分钟） | 不经用户账户签名就扣费 |
| EntryPoint | v0.7 | 唯一能调用 `validatePaymasterUserOp` / `postOp` 的地址 | — |

**72 小时排队（排队期内 owner 可撤销，到期后任何人执行）**：`setParams` 中任何提高收费或收紧规则的修改（费率、最低手续费、最低订单金额、基础 gas 费、领取 gas 费、争议费倍数上调，以及 `resolveTimeout`、`proposalWindow` 任何变化）；运行中的免费活动变得不那么优惠（含提前结束）；新增代币；更换拍卖签名人；新增市场。

**立即生效**：所有放宽收费的参数修改；单笔上限与全站在押上限（调高调低都立即生效，只影响新订单）；暂停 / 恢复；停用代币；开始或延长免费活动。

**合约写死、任何人都改不了的上限**（`EscrowConfig` 常量）：

| 常量 | 值 | 常量 | 值 |
| --- | --- | --- | --- |
| `MAX_FEE_BPS` | 10%（1000） | `MAX_DISPUTE_MUL` / `MIN_DISPUTE_MUL` | 5 倍 / 0.5 倍 |
| `MAX_MIN_FEE` | 5 USDT | `MIN_RESOLVE_TIMEOUT` – `MAX_RESOLVE_TIMEOUT` | 14 – 60 天 |
| `MIN_MIN_ORDER` – `MAX_MIN_ORDER` | 1 – 100 USDT | `MIN_PROPOSAL_WINDOW` – `MAX_PROPOSAL_WINDOW` | 1 – 7 天 |
| `MAX_ORDER_CAP` | 100 000 USDT | `MAX_CAMPAIGN` | 90 天 |
| `MAX_TOTAL_LOCKED_CAP` | 1 000 000 USDT | `MAX_GAS_SUBSIDY` | 10 000 单 |
| `MAX_BASE_GAS`（也限领取 gas 费） | 1 USDT | `DELAY` | 72 小时 |

市场常量：期限窗口 1–30 天（Escrow）、开始期限 1–7 天与总时长 ≤ 365 天、每期净本金 ≥ 0.1 USDT、领取间隔 24 小时（StreamEscrow）、卖家延长确认期累计 ≤ 30 天、买家紧急顺延 24 小时（最后 6 小时内 1 次）、仲裁延期单次 ≤ 7 天 / 累计 ≤ 14 天、人民币汇率 5.0000–10.0000。Paymaster：`MAX_FEE` = 1 USDT、`MAX_VALIDITY` = 10 分钟。

## 6. 不变量与对应测试

测试位置：`contracts/test/`（单元、模糊、不变量）与 `contracts/test/symbolic/`（Halmos，结果见本节末尾）。

| Id | 不变量 | 单元 / 模糊 / 不变量测试 | 符号检查 |
| --- | --- | --- | --- |
| I1 | 资金守恒：每单付入 = 付给买家 + 卖家 + 收款地址 + 仍在合约；合约余额 = 未结订单 + 保证金 + 待领 credit | `EscrowInvariant`、`StreamInvariant` 的 `invariant_conservation`、`invariant_balance(s)MatchObligations`；`EscrowFuzz` / `StreamFuzz` 的 `*_conserves` | Escrow / Stream 每种结局的 `_assertAllSettled` |
| I2 | 订单资金只到买家、卖家、收款地址 | 同上（守恒只统计这三类账户）；`test_frozenRecipient_creditAndWithdraw`、`test_settle_frozenBuyerCredited` | 同上（`_allowedHoldings`：总供应不变时，这几个账户合计不变 ⇒ 其他地址没收到钱） |
| I3 | 单笔 ≤ `orderCap`；新订单与保证金不让 `totalLocked` 超过 `totalLockedCap`；`totalLocked` = 未结订单 + 保证金 | `test_totalLockedCap_blocksNewOrdersAndBonds_notDisputes`、`test_settle_cny_orderLimitsAfterConversion`、`test_settle_cny_totalLockedCapAfterConversion`、`invariant_totalLockedMatches` | `EscrowSymbolic.check_caps`、`StreamSymbolic.check_caps` |
| I4 | 费用不超过报价签的上限 | `test_maxMinFee_protectsOldOffer`、`test_revert_create_offerChecks`、`test_revert_feeLimits`、`test_createAndFund_snapshotSurvivesParamChange` | `EscrowSymbolic.check_feeWithinSignedCaps` |
| I5 | 每单平台最多收 gas + {手续费、争议费} 之一 | `test_resolve_buyerDisputerLoses_onlyOneCharge`、`testFuzz_feesCharged`、`test_feesCharged_examples` | `FeeMathSymbolic.check_feesCharged_*`；Escrow / Stream `check_resolve` |
| I6 | 每个未结订单都有别人挡不住的出口；条件满足时结算不回滚 | `test_claimTimeout_*`、`test_resolveTimeout_*`、`test_pause_keepsExistingOrdersWorking`、`test_token_disableImmediate_existingOrdersSettle` | 每个结局检查里 `assert(_call(...))`（结算调用必须成功） |
| I7 | 分期：已领取 ≤ 已解锁 ≤ 净本金；解锁单调、争议后冻结、到期等于净本金 | `invariant_unlockBounds`、`testFuzz_disputeFreezesUnlock`、`testFuzz_streamUnlockedMonotonic`、`test_streamVectors` | `FeeMathSymbolic.check_streamUnlocked_monotonic`；`StreamSymbolic.check_unlock_*`、`check_claim` |
| I8 | 保证金：每个卖家已锁定 = 其未结订单保证金之和；只能取 / 用未锁定部分；赔付只给该单买家 | `invariant_bondsLockedByOpenOrders`（两份）、`EscrowBond.t.sol` 全部 | Escrow / Stream `check_claimTimeout`、`check_resolve` |
| I9 | 拍卖每场只成交一次、付款人是赢家、成交价 ≥ 保留价、人民币按卖家签名汇率向上取整 | `EscrowAuction.t.sol` 全部、`test_cnyToUsdtVectors`、`testFuzz_cnyFenToUsdtRoundsUp`、`testFuzz_settle_cny_neverUnderpaysSeller` | `EscrowSymbolic.check_auction`；`FeeMathSymbolic.check_cnyFenToUsdt_*` |
| I10 | 管理员动不了钱：参数建单快照、收紧排队 72 小时、硬上限、暂停只挡新订单 | `EscrowConfig.t.sol` 全部、`test_createAndFund_snapshotSurvivesParamChange` | `check_feeWithinSignedCaps`（任意参数 + 任意活动） |
| I11 | 报价防重放：EIP-712 域、指定买家、成交次数、过期、`cancelOffer` | `test_createAndFund_maxFills`、`test_createAndFund_designatedBuyer`、`test_revert_create_signatureBoundToChain`、`test_revert_create_cancelledNonce`、`OfferHashVector.t.sol`、`EscrowSmartAccount.t.sol` | —（签名正确性不做符号检查，见下） |
| I12 | Paymaster：只在签名有效时扣费、扣的正是签名费用且 ≤ 1 USDT、窗口非零且 ≤ 10 分钟、费用只去金库、只有 EntryPoint 能触发 | `FixedFeePaymaster.t.sol` 全部 | `PaymasterSymbolic` 全部 |
| — | 费用规则与共享测试向量一致（ADR 0002） | `FeeMath.t.sol` 的 `test_*Vectors` | `FeeMathSymbolic.check_feeTotal`、`check_split`、`check_claimNet` |

### 符号检查（Halmos）

工具：Halmos 0.3.3（`uvx --from halmos==0.3.3`），求解器 cvc5 1.2.1 的 `cvc5-int` 模式（位向量转整数推理），由各 spec 合约的 `@custom:halmos` 标注指定；默认的 yices、bitwuzla、z3 在 256 位乘除（含 Solidity 溢出检查 `x*c/c == x`）上全部超时。命令：`contracts/script/halmos.sh`（全部），或带 halmos 参数单跑。

建模（`test/symbolic/SymbolicBase.sol`）：按 plan 第 9 节默认参数部署 EscrowConfig + Escrow + StreamEscrow；卖家与拍卖签名人是接受任何签名的 ERC-1271 账户（签名正确性由单元测试覆盖）；只有买家、卖家、收款地址和两个市场持有代币，总供应量不变，所以「这几个账户合计不变」等价于「没有别的地址收到钱」。每个检查只建一笔订单，每种结局一个检查；结局调用用低级 call 并断言成功（条件满足时不能回滚 = 资金不会卡住）。

界限：符号化金额会让按比例费用与其溢出检查变成非线性、求解器超时，所以生命周期检查的金额取自小网格（`_gridAmount`：5、99.999999、123.456789、200 USDT；比例 `_gridBps`：0、0.01%、50%、99.99%、100%；分期期数 1/3/12/365，经过时间取 0、一期前 1 秒、一期、过半、结束前 1 秒）。付费方、谁发起争议、是否已发货 / 开始、提议方、时间偏移等保持符号化。`FeeMath` 的检查覆盖完整可达值域（uint40 ≥ 合约硬上限 1e11）。

| 检查 | 性质 | 结果（耗时） |
| --- | --- | --- |
| FeeMath `check_split` / `check_feeTotal` / `check_claimNet` | 拆分之和等于总额；费用公式；领取净额 | 证明（< 1 s） |
| FeeMath `check_streamUnlocked_monotonic` | 解锁单调、≤ 净本金、到期等于净本金（全值域） | 证明（0.2 s） |
| FeeMath `check_feesCharged_total` / `_noDispute` | 留存争议费替代手续费：收取 = max(0, 应收 − 留存)；无争议时精确按比例 | 证明（< 1 s） |
| FeeMath `check_feesCharged_buyerPartCapped32` | 买家部分不超过按比例值（uint32，≥ 4294 USDT） | 证明（243 s） |
| FeeMath `check_feesCharged_sellerPartCapped32` / `_partsCapped`（uint40） | 卖家部分 / 两部分不超过按比例值 | **超时**（由 `testFuzz_feesCharged` 与下面各结局检查间接覆盖：结算不下溢依赖此性质） |
| FeeMath `check_cnyFenToUsdt_ceil` / `_fixedRates` | 人民币换算向上取整、多付 < 1 单位 | **超时**（Solady `fullMulDivUp` 的 512 位路径无法排除；由向量、`testFuzz_cnyFenToUsdtRoundsUp`、`testFuzz_settle_cny_neverUnderpaysSeller` 覆盖） |
| Paymaster `check_validate` | 接受时扣的正是签名费用且 ≤ 1 USDT、窗口非零且 ≤ 10 分钟、原样上报；拒绝时不动钱（签名任意、ecrecover 未解释 = 等同私钥被盗） | 证明（0.2 s） |
| Paymaster `check_validate_onlyEntryPoint` / `check_sweep_onlyToTreasury` / `check_setSigner_onlyOwner` | 只有 EntryPoint 能触发验证；费用只去金库；只有运营 Safe 能换签名人 | 证明（< 1 s） |
| Escrow `check_release` / `check_refund` / `check_resolveTimeout` | 结局可执行、守恒、只付三方、金额精确（放款 / 退款）、平台只收费用池 | 有界证明（12 s / 7 s / 42 s） |
| Escrow `check_claimTimeout` / `check_acceptEnd` | 超时放款或退款 + 保证金赔付比例；任意比例和解（含争议中） | 有界证明（318 s / 236 s） |
| Escrow `check_resolve` | 任意裁决可执行；买家至少拿到判给的本金与保证金份额（减去卖家那份未被覆盖的 gas，L11）；平台最多收 gas + {手续费、争议费}之一 | 首次运行发现**规格**反例（全额判给买家时卖家 gas 份额由买家退款承担，即 L11），规格已改；改后重跑未在时限内完成：**有界，结果待复跑** |
| Escrow `check_caps` | 任意保证金存入 + 任意两笔订单（金额全符号化）后单笔 ≤ 200、`totalLocked` ≤ 5000 USDT | 证明（76 s） |
| Escrow `check_auction` | USDT 拍卖成交价 ≥ 保留价且金额等于成交价；同一场拍卖换任何卖家授权（保留价、币种、汇率）都不能再成交（价格全符号化） | 证明（0.3 s） |
| Escrow `check_auction_cny` | 人民币拍卖按 `_gridRate` 汇率向上取整、多付 < 1 单位 | **超时**（同 FeeMath 换算；单元与模糊测试覆盖） |
| Escrow `check_feeWithinSignedCaps` | 任意参数（硬上限内）+ 任意活动下费用不超过报价签的上限 | 网格版在时限内未跑完：**待复跑**（全符号版超时；`test_maxMinFee_*`、`test_revert_create_offerChecks` 覆盖） |
| Stream `check_claimTimeout` / `check_unlock_frozenByDispute` | 未开始超时全额退款 + 赔付；争议后解锁冻结 | 有界证明（328 s / 143 s） |
| Stream `check_finalize` / `check_refund` | 到期结清 / 卖家退款 | 首次运行发现**规格**反例（单期分期「一期」即到期，领取已结清订单），网格已改；**待复跑** |
| Stream 其余（`check_claim`、`check_acceptEnd`、`check_resolve`、`check_resolveTimeout`、`check_caps`、`check_unlock_monotonic`） | 同 Escrow 对应性质 | 时限内未运行：**待复跑**（符号时间版的 `check_unlock_monotonic` 曾超时；单调性已在 FeeMath 层证明） |

非空性验证（`assert(false)` 探针与故意改坏的合约副本，均在临时副本中运行）：FeeMath 11 个、Paymaster 4 个检查加上探针后全部给出反例（路径可达）；改坏的变体都被抓到：去掉 `validUntil == 0` 检查 → `check_validate` 反例；解锁到期返回 `net + 1` → `check_streamUnlocked_monotonic` 反例；人民币换算改成向下取整 → `check_cnyFenToUsdt_ceil_fixedRates` 反例（正确版本超时，说明超时不是「没约束」）。Escrow / Stream 检查的反例本身（上表两处规格反例）也说明这些路径可达。

结论：没有发现合约缺陷；两处反例都是规格写得比设计更严，已修正规格。

## 7. 变异测试

工具：Foundry 1.8.3 自带的 `forge test --mutate`（版本随 CI 固定，不用另装；懂 remappings 与项目结构，并行 worker，按「文件 + 测试」哈希缓存结果）。没选 Gambit / vertigo-rs：要另装、另配编译器，维护活跃度不如 Foundry 本体；slither-mutate 只能串行原地改文件。命令：`contracts/script/mutation.sh src/<File>.sol...`（在临时副本里跑，不碰工作区）。

两阶段：

1. **筛选**（`script/mutation.sh`）：跳过不变量套件、模糊 64 次。不变量被打破时 forge 会做序列收缩，每个这样的变异要几分钟。
2. **确认**：每个存活变异再跑完整套件（模糊 1000 次 + 全部不变量，关闭收缩）；仍存活的逐个判断是否等价，不等价的补测试杀掉。

结果（第一轮筛选用当时的测试；「最终」= 补测试后的第二阶段）：

| 合约 | 生成 | 编译失败 | 自适应跳过 | 测试 | 筛选杀死 | 存活 → 补测试杀死 | 等价 | 非等价变异得分 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| FeeMath | 311 | 23 | 21 | 267 | 257 | 0 | 10 | 100%（257/257） |
| MarketBase | 62 | 0 | 35 | 27 | 21 | 3 | 3 | 100%（24/24） |
| FixedFeePaymaster | 103 | 7 | 49 | 47 | 41 | 4 | 2 | 100%（45/45） |
| EscrowConfig | 602 | 22 | 367 | 213 | 181 | 15 | 17 | 100%（196/196） |
| Escrow | 1164 | 52 | 437 | 675 | 595 | 56 | 24 | 100%（651/651） |
| StreamEscrow | 1007 | 50 | 350 | 607 | 540 | 43 | 24 | 100%（583/583） |

「自适应跳过」是 forge 在同一表达式（或其子表达式）上已有变异存活时不再测的部分。补测试后用最终测试重跑筛选（会测到这些子表达式）因时限中断，**待复跑**：`script/mutation.sh src/Escrow.sol`、`src/StreamEscrow.sol`、其余四个文件各一批，每批约 30–40 分钟（2 个 worker、≤ 4 GB）。

补的测试按类别（全部在原有测试文件里）：

- 访问控制只比较相等：所有「只有买家 / 卖家 / 仲裁」的函数用地址 `1` 和 `2^160−1` 各调一次（`test_revert_notParty_anyAddress`，两个市场）；指定买家、成交证明的 `authHash` / `winner` 两侧取值；买家地址大于卖家的正常下单
- 已结算订单不能再被 `resolve`、`resolveTimeout`、`proposeEnd`、`acceptEnd`、`dispute` 触碰（`test_revert_settledOrderIsFinal`，`_disputed` / `_open` 的状态比较若写成 `<` 会重复结算）
- 期限边界：恰在期限可操作、远早于期限不可操作（`claimTimeout`、`resolveTimeout`、领取间隔、`finalize`、`markDelivered`、`dispute`、`acceptEnd`、买家紧急顺延窗口内任意时刻）
- 时间戳改为真实值（`EscrowBase.START_TIME`）：在 `block.timestamp = 1` 时 `t + window` 与 `t | window` 结果相同
- 保证金记账：多次存入、多笔订单锁定 / 逐笔释放、市场部分释放与 100% 赔付、恰好等于争议费的可用保证金
- 等号边界：费用恰等于金额、恰好等于争议费的已解锁余额、开始期限 1 天 / 7 天
- 「两个条件同时不满足」的组合：参数、活动、报价检查，以及「已结算且超过延期上限」的订单不被延期
- 代付：零费用不碰代币（ERC-7562）、恰好 80 字节无签名、`validUntil = 0` 且 `validAfter > 0`
- 收款与事件：credit 累加、转入多于请求的代币也拒绝（`BonusToken`）、无保证金的分期订单不调用保证金池、队列事件里的种类编号、过期活动 / 其他范围的活动不改费率、5 位小数代币被拒、启用过的代币停用后需重新排队

等价变异（共 80 个，逐个确认过完整套件也杀不掉）：

- 无符号 / 枚举比较：`x != 0` ↔ `x > 0`、`x == 0` ↔ `x <= 0`、地址与 0 比较、首个 / 末个枚举值的 `==` ↔ `<=` / `>=`、依次判断的 `if (i == Hour) … if (i == Day)`（47 个）
- 循环与自增：`i < n` ↔ `i != n`、`++x` ↔ `x++`（7 个）
- `||` 链末两项互斥（例如 `w < MIN` 与 `w > MAX`、`secs == 0` 与 `secs > MAX`、`periods == 0` 与 `periods × 间隔 > MAX`）时 `||` → `!=`（11 个）
- 边界处两边结果相同：`fee >= minFee` ↔ `>`、`elapsed >= periods` ↔ `>`、`claimable <= claimGas` ↔ `<`、`fee > retained` ↔ `>=`、`s > sellerCost` ↔ `>=`、`retained > 0` ↔ `>= 0`（被 `fee > 0` 保护）（6 个）
- 不可达或结果相同的防御性检查：卖家地址为 0（验签先失败）、拍卖签名人为 0（Solady 对 0 地址返回无效，同样 `BadSignature`）、`fills > maxFills`（不可达）、`subsidyUsed > cap`（不可达）、`_running` 中 start > now ≥ end（不可达）、保证金为 0 时的锁定 / 释放 / 推送（无状态与事件差异）（9 个）

测试基础设施问题（变异测试顺带发现，已处理）：

- `dynamic_test_linking`（Foundry 1.8 默认开启）让 `vm.expectRevert(...); new X(...)` 只检查第一个用例：三个构造函数零地址测试此前实际上只测了第一个参数。`foundry.toml` 已关闭该选项（第 2 节）
- 测试读取 `../test-vectors`；forge 把项目复制到 `$TMPDIR/forge_mutation_*` 运行变异，向量缺失会让每个变异都「被杀死」（第一次 FeeMath 得分 100% 是假象）。`script/mutation.sh` 在临时目录旁放了向量链接
- 把循环改成死循环的变异会在 forge 默认的极大 gas 上限下用光内存；脚本把每个测试的 gas 上限设为 5e7
- Forge 1.8 把 `@custom:halmos` 当作内联配置解析，`300s` 这样带单位的值会让 `forge test` 直接报错；标注改用毫秒数字

## 8. 已知限制与接受的风险

下面这些是有意的设计取舍或第三方风险，AI 审计与赏金不再重复计入（除非能证明后果比这里写的更严重）。

| # | 限制 / 风险 | 后果 | 对策 |
| --- | --- | --- | --- |
| L1 | 争议费不受全站在押上限约束（`lockFunds(fee, false)`，避免上限挡住争议） | `totalLocked` 可以超过 `totalLockedCap`，超出量不超过每个未结订单一份争议费（≤ 5 倍手续费） | 限损口径是「订单 + 保证金 ≤ 上限，另加争议费」；`test_totalLockedCap_blocksNewOrdersAndBonds_notDisputes` 覆盖 |
| L2 | **全站在押上限可被占满**：任何人用两个自己的账户互相下单（卖家随后 `refund`，不收手续费），或存入保证金，就能把 `totalLocked` 推到上限 | 期间全站不能建新单、不能存保证金；不会损失任何人的资金，攻击者只付资金占用与代付费 | 监控 `totalLocked` 与大额保证金；金库 Safe 可立即调整上限（上限调整不排队）或暂停；forum-sponsor 对异常账户拒绝代付。合约内按账户限额属于设计变更，冻结版本不做 |
| L3 | `EscrowConfig` 与 Paymaster 继承了 `renounceOwnership`；Paymaster 的 owner 是单步移交（`Ownable`） | 误调用后参数永久冻结、不能再暂停（已有订单不受影响）；Paymaster 押金取不回、签名人不能轮换 | Safe 交易审查清单禁止这两个调用；Paymaster 移交 owner 前先在测试网演练 |
| L4 | 暂停只挡新订单（`checkout`），不挡 `depositBond` / `withdrawBond` | 暂停期间卖家仍可存取保证金 | 保证金存入仍受代币白名单与全站上限约束；需要时 `disableToken` |
| L5 | USD₮0 发行方风险：冻结某个合约地址、暂停代币、通过代理升级改变语义 | 冻结个人地址只影响本人（领取式转账，T-08）；冻结或暂停 Escrow / StreamEscrow 地址会让所有出口失败，资金在解冻前无法取回 | 接受；全站上限把最坏损失封顶 |
| L6 | 仲裁 Safe（2/3）可以把争议订单的本金按任意比例判给买家或卖家（T-15） | 勾结时一方的钱可被判给另一方，但到不了第三方 | 治理措施（签名人当期不审案、大额双人、申诉、公开理由）；合约只保证钱只在买卖双方之间分 |
| L7 | 拍卖签名私钥被盗（T-20） | 可为任何卖家签过授权的拍卖签发成交证明：以不低于保留价卖给攻击者指定的赢家（赢家自己付钱），正当的最高出价者拿不到；同一场仍只能成交一次 | 私钥只在 forum-sponsor；更换签名人排队 72 小时，紧急时金库 Safe 可暂停 |
| L8 | Paymaster 代付签名私钥被盗（T-11） | 可为任意操作签代付，耗尽 ETH 押金（约 2 周用量）；不能在没有用户账户签名的情况下扣用户 USDT，每笔最多 1 USDT | 押金只放 2 周用量、余额告警；运营 Safe 立即 `setSigner` |
| L9 | Paymaster 不在链上检查 `validAfter` 是否等于当前时间（验证阶段不能读 `block.timestamp`，ERC-7562） | forum-sponsor 可以签一个将来才生效的 10 分钟窗口 | forum-sponsor 固定 `validAfter = now`；EntryPoint 按窗口检查 |
| L10 | 舍入：手续费、按比例分配、分期解锁一律向下取整；「各半」时多出的 1 个最小单位归卖家；人民币换算向上取整 | 每笔最多 1 个最小单位的偏差（买家在人民币拍卖里最多多付 1 个最小单位） | 与 `test-vectors/fees.json` 规则一致（ADR 0002） |
| L11 | 平台代付过操作后（`platformPaid`），即使全额退款也扣基础 gas 费，卖家承担的那份由买家的退款承担 | 全额退款时买家少拿卖家那份 gas 费（默认 0.05 USDT） | plan 第 6 节「费用池与 gas 费先留再分」的既定规则 |
| L12 | 依赖 `block.timestamp`；排序器停摆期间期限照常流逝（T-19） | 期限可能被动到期 | 所有窗口 ≥ 24 小时；仲裁 Safe `extendDeadlines` |
| L13 | 市场一经加入不能移除；代币停用立即生效但不影响已有订单 | 一个有问题的市场只能靠暂停与迁移处理 | 新增市场排队 72 小时、公告 |
| L14 | 符号检查是有界的（第 6 节）：一次只建一笔订单、金额与裁决比例取代表值、签名用接受一切的 ERC-1271 模拟账户 | 不是完整的形式化证明 | 由单元、模糊、不变量测试与变异测试补足；签名正确性由单元测试（含 ERC-1271 / ERC-6492）覆盖 |

## 9. 部署参数

`contracts/script/Deploy.s.sol`（EscrowConfig + Escrow + StreamEscrow；构造时登记两个市场）：

| 参数 | 主网值 | 说明 |
| --- | --- | --- |
| `TOKEN` | USD₮0 `0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9` | 测试网用 `MockUSDT` |
| `OWNER` | 金库 Safe（2/3） | EscrowConfig owner |
| `ARBITER` | 仲裁 Safe（2/3，≠ 金库） | 脚本在主网强制检查 |
| `FEE_RECIPIENT` | 金库 Safe | 不可变 |
| `AUCTION_SIGNER` | forum-sponsor 拍卖签名地址；不设 = 关闭拍卖 | 之后更换须排队 72 小时 |
| `feeBps` / `minFee` / `minOrder` | 100（1%）/ 1 USDT / 5 USDT | plan 第 9 节默认值 |
| `orderCap` / `totalLockedCap` | 200 USDT / 5000 USDT | ADR 0007 上线初期限损 |
| `baseGas` / `claimGas` | 0.1 / 0.02 USDT | 测试网实测后校准（plan 第 17 节待决） |
| `disputeMulBps` | 10 000（1 倍） | |
| `resolveTimeout` / `proposalWindow` | 30 天 / 72 小时 | |
| Solady ERC-6492 验证器 | `0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e` | 主网必须已存在，脚本不代为部署 |

`FixedFeePaymaster(entryPoint, token, treasury, signer, owner)`：EntryPoint v0.7、USD₮0、金库 Safe、forum-sponsor 代付签名地址、运营 Safe；部署后 `deposit()` 约 2 周用量的 ETH，若要用公共 Bundler 备用再 `addStake`（`deploy/dev/aa.sh` 的本地值：押金 10 ETH、质押 1 ETH、解质押延迟 1 天，主网按实测用量定）。代付费默认 0.05 USDT 由 forum-sponsor 签入，合约只限 ≤ 1 USDT。

部署后核对：第 1 节字节码哈希、`config.params()` 与上表一致、`config.isMarket(escrow) && config.isMarket(stream)`、`escrow.arbiter()` / `feeRecipient()`、`config.owner()`、Paymaster `owner()` / `signer()` / `treasury()` / `token()`、`entryPoint.balanceOf(paymaster)`。

## 10. 冻结门禁（每次冻结候选都要全部通过）

```sh
cd contracts
forge fmt --check && forge build --sizes && forge test
forge coverage --report lcov --report summary   # CI 用 awk 检查 src/ 行覆盖率 100%
uvx --from slither-analyzer==0.11.6 slither . --exclude-dependencies --filter-paths "lib/|test/" --fail-high
script/halmos.sh                                  # 第 6 节符号检查，完整一轮 1–2 小时
forge test --mutate src/<File>.sol                # 第 7 节，按文件分批
script/audit-bundle.sh                            # 生成 AI 审计包
```

CI（`.github/workflows/ci.yml` 的 `contracts` 作业）跑前四项，并对 `script/` 下三个审计脚本跑 shellcheck、生成一次审计包；Halmos 与变异测试远超 10 分钟预算，作为冻结前的手动门禁（结果写回第 6、7 节）。

## 11. 冻结后的变更流程

1. 冻结后 `contracts/src/` 只接受：经验证的审计 / 赏金发现的修复（先有失败测试）、编译器或依赖的安全修复。其他改动（含重构、注释以外的格式变化）等下一次冻结
2. 每个修复单独一个 PR：失败测试 → 最小修复 → 第 10 节全部门禁；PR 说明写明影响的不变量（第 6 节 Id）与威胁条目（`T-xx`）
3. 改动过的合约重跑变异测试（第 7 节命令），新的存活变异要么用测试杀掉、要么写进等价变异表；改到的性质重跑对应符号检查
4. 更新本清单第 1 节哈希、第 6–8 节结果，重新生成审计包，并对改动部分跑一轮多方 AI 复审（[ai-audit-pack.md](ai-audit-pack.md) 第 7 节「Re-run」）
5. 已经部署到主网的合约不可升级：修复版本按 plan 第 13 节合约迁移流程部署新合约（旧合约暂停新订单，旧订单走完）；公开审查期内的修复要重新开始计算剩余审查期中未被覆盖的部分（站长决定是否顺延）
6. 改动依赖版本（OpenZeppelin、Solady、account-abstraction）等同修改合约，同样走以上流程，并更新第 3 节
