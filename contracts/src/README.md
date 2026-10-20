# 合约规格清单（plan-v3 第 6 节）

两份不可升级合约 `Escrow`（一次性）与 `StreamEscrow`（分期），共用 `FeeMath`、签名与权限代码，一起审计。第 12 周（2027-01-04）冻结送审。

## 已完成

- [x] `libraries/FeeMath.sol`：手续费（含活动费率）、承担方拆分、买家应付、争议费、分期净本金 / 解锁 / 领取净额、人民币成交价换算 USDT（`cnyFenToUsdt`）；过 `test-vectors/fees.json`
- [x] `MockUSDT.sol`：仅测试网，6 位小数
- [x] `StreamEscrow.sol`：分期担保（第 4 轮），经构造时登记为 `EscrowConfig` 市场，保证金走 `Escrow` 统一池
- [x] `MarketBase.sol`：两份合约共用的报价计数 / 作废 / ERC-6492 验签与领取式转账（`withdraw`、`cancelOffer`）；`FeeMath.feesCharged` 共用按比例收费与争议费抵扣

## 合约结构（第 3 轮起）

- `EscrowConfig`：金库 Safe 拥有的全部可调设置，`Escrow` 与 `StreamEscrow` 共用（参数与 72h 排队、暂停、免费活动、代币白名单、拍卖签名人、市场列表、全局在押资金上限计数）。建单时经 `checkout` 取快照；`claimGas`（分期每次领取 gas 费，默认 0.02 USDT，0–1 USDT）也在参数里，调高排队 72h，gas 补贴订单同时免领取 gas 费。
- `Escrow`：一次性担保 + 统一卖家保证金池；没有 owner。所有可调项都在 `EscrowConfig`，`Escrow` 不可升级、无管理员函数。
- `StreamEscrow`：分期担保；仲裁 Safe、收款地址与 `EscrowConfig` 均从 `Escrow` 读取，两份合约不会配错。免费活动范围「担保」（`SCOPE_ESCROW`）同时覆盖一次性与分期。
- 拆分原因：功能全部放进一份合约会超过 24 KB 代码上限；现在 Escrow 22.5 KB、StreamEscrow 16.9 KB、EscrowConfig 11.3 KB。

## 人民币计价拍卖（ADR 0011）

- `AuctionAuth` 多 `currency`（0 = USDT、1 = 人民币，其他值拒绝）与 `cnyPerUsdt`（每 1 USDT 合多少元，4 位小数定点数，7.2000 = 72000），两者都在卖家 EIP-712 签名里，平台改不了
- USDT 拍卖要求 `cnyPerUsdt == 0`；人民币拍卖要求 `cnyPerUsdt` 在写死的 [5.0000, 10.0000]（`MIN_CNY_PER_USDT` / `MAX_CNY_PER_USDT`）内，防止把 7.2 填成 72 或 0.72
- 人民币拍卖的 `reservePrice` 与成交证明的 `price` 都以分计；先按拍卖币种检查 `price ≥ reservePrice`，再换算：`amount = ceil(price × 10^8 / cnyPerUsdt)`（10^8 = USDT 6 位小数 × 汇率 4 位小数 ÷ 每元 100 分；白名单只收 6 位小数代币），向上取整，卖家所得不低于签名汇率换算值，买家最多多付 1 个最小单位；乘法用 512 位中间值（Solady `fullMulDivUp`），不会因溢出算错（结果超出 uint256 时回滚）
- 换算后的 USDT 金额照常走手续费、最低订单 5 USDT、单笔 200 / 全站 5000 上限与 `FeesExceedAmount` 检查
- 成交证明不变（价格用拍卖币种，经 `authHash` 绑定卖家授权）；`AuctionSettled` 事件带 `currency`、`cnyPerUsdt`、`price`（拍卖币种）与 `amount`（USDT）

## 分期（StreamEscrow）规则

- 报价多 `interval`（每小时 / 每天 / 每周 / 每月 = 30 天）、`periods`、`startWindow`（24h–7 天）、`maxClaimGas`；总时长 ≤ 365 天；每期可解锁净本金 ≥ 0.1 USDT（按净本金 ÷ 期数校验）
- 可解锁净本金 `net` = 金额 − 卖家承担手续费 − 卖家承担基础 gas；费用池单独存放。结算以「卖家在 `net` 中的总份额 `s`（含已领取）」表示：手续费按 `s / net` 收取，平台代付过则基础 gas 全收，其余退买家
- `start` 后按整周期解锁（`FeeMath.streamUnlocked`）；争议后一律按 `disputedAt` 计算，不再增长
- `claim`：可领 > `claimGas`、距上次 ≥ 24h，当次扣 `claimGas` 给收款地址；流结束后 `claim` 等同 `finalize`（任何人可调，不扣领取 gas）
- `refund`：卖家退还全部未领取部分，只按已领取部分收手续费；`claimTimeout`：开始期限已过未开始 → 全额退款并按 `lateSlashBps` 赔付保证金
- `proposeEnd` / `acceptEnd`：`toBuyer` ≤ 未领取部分（接受时再校验，防止提议后领取导致不够付）
- `dispute`：开始前（≤ 开始期限）或流结束前；卖家发起时若本单已解锁未领取 ≥ 争议费则直接从中划扣，否则从钱包；`disputeWithBond` 经 `Escrow.takeBond` 从可用保证金划转。卖家争议费退回时一律打到卖家钱包
- `resolve`：按 `toBuyerBps` 拆分全部未领取部分（含争议前已解锁未领取）；`resolveTimeout`：未开始 → 全额退款；已开始 → 争议时已解锁部分归卖家，其余各 50%；争议费全退
- `extendDeadlines`：只延长未开始订单的开始期限（单次 ≤ 7 天、累计 ≤ 14 天）

## 未做（下一轮）

- [ ] （无；StreamEscrow 已完成，等待第 12 周冻结送审）

## 状态机

`createAndFund` → `Funded` →（`Disputed`）→ `Released` | `Refunded` | `Settled`。没有「已创建未付款」状态。

## 函数（两份合约通用，除注明外）

- [x] `createAndFund(offer, sellerSig)` 买家：EIP-712 + ERC-6492 验卖家报价，按实际到账余额记账，同交易锁定卖家保证金
- [x] `markDelivered`（Escrow）/ `start`（StreamEscrow）卖家
- [x] `release` 买家（Escrow）
- [x] `claim` 卖家（StreamEscrow）：可领 > 0.02 USDT，间隔 ≥ 24h，当次扣领取 gas 费；`finalize`（StreamEscrow）任何人：到期后结清余额
- [x] `refund` 卖家主动全额退款（不收手续费）
- [x] `proposeEnd` / `acceptEnd` 协商和解，72h 失效
- [x] `dispute` 发起方预付争议费；卖家可用已解锁未领余额或可用保证金抵扣；StreamEscrow 记录 `disputedAt`
- [x] `resolve(orderId, toBuyerBps, bondToBuyerBps)` 仅仲裁 Safe
- [x] `claimTimeout` 任何人：确认期满放款 / 未按期发货退款 / 分期未开始退款
- [x] `resolveTimeout` 任何人：争议 30 天未裁决按默认规则
- [x] `withdraw(token, to)` 领取式转账余额
- [x] `cancelOffer(nonce)` 卖家
- [x] `extendConfirm(orderId, secs)` 卖家（累计上限）；买家最后 6h 内免费紧急顺延 1 次 24h
- [x] `createFromAuction(授权, 卖家签名, 成交证明, 平台签名)` 拍卖赢家；每场只成交一次、成交价 ≥ 保留价（成交证明带 `authHash` 绑定卖家授权）；人民币拍卖按卖家签名汇率向上取整换算 USDT（见上「人民币计价拍卖」）
- [x] `extendDeadlines(orderIds, secs)` 仲裁 Safe：单次 ≤ 7 天、累计 ≤ 14 天
- [x] `depositBond` / `withdrawBond`（Escrow 统一保证金池，市场经 `lockBond` / `releaseBond` 锁定 / 解锁 / 赔付，经 `takeBond` 划走卖家可用保证金付本市场争议费）
- [x] `pause` 只阻止新建订单

## 参数（owner = 金库 Safe）

- [x] 参数排队：收紧 72h 生效、可撤销；放宽立即生效；所有参数建单时写入订单快照
- [x] 硬上限：费率 ≤ 10%、最低手续费 ≤ 5 USDT、最低订单 1–100 USDT、单笔上限 ≤ 100000 USDT、gas 费 ≤ 1 USDT、争议费倍数 0.5–5、超时兜底 14–60 天、gas 补贴 ≤ 10000 单
- [x] 免费活动：开始 / 结束 / 活动费率 / 范围，单次 ≤ 90 天；gas 补贴按订单数计数
- [x] 代币白名单：上线只开 USD₮0，新增经金库 Safe 排队 72h（只收 6 位小数代币）
- [x] 报价最高费率 / 最高 gas 费 / 最高最低手续费保护
- [x] 全局在押资金上限 `totalLockedCap`（默认 5000 USDT，硬上限 100 万）：在押 = 未结订单持有 + 卖家保证金；新订单与保证金存入超限即拒绝，争议费不受限；单笔上限默认 200 USDT。两个上限调高调低都立即生效（只影响新订单）

## 部署

- `script/Deploy.s.sol`：`TOKEN` 不设时（只限测试网与本地链）先部署 `MockUSDT`；再部署 `EscrowConfig`（预测 Escrow 与 StreamEscrow 地址登记为市场）、`Escrow`、`StreamEscrow`，设了 `PAYMASTER_SIGNER` 时最后部署 `FixedFeePaymaster`（owner = `PAYMASTER_OWNER` 运营 Safe，treasury = 金库 Safe）；测试网缺 Solady ERC-6492 验证器时经 CREATE2 部署，Arbitrum One 上缺失则中止（需单独、有意识地部署）。测试网用 `deploy/testnet/testnet.sh` 调用它（docs/runbooks/testnet-deploy.md）。
- `foundry.toml` 已配 `arbitrum_sepolia` / `arbitrum` 的 RPC 与 Arbiscan（从环境变量读取）。

## 安全要求

- [x] OpenZeppelin `SafeERC20` / `ReentrancyGuard` / `EIP712` / `SignatureChecker` + Solady ERC-6492；先改状态再转账
- [x] 签名域绑定 chainId + 合约地址；按报价哈希计数成交次数
- [x] 无管理员提款、无代理升级、无 `selfdestruct`
- [x] 不变量：已领取 + 已退款 + 费用 + 合约余额 = 订单总额；合约余额 = 未结订单 + 保证金 + 待领余额；`totalLocked` = 未结订单 + 保证金；每个卖家已锁定保证金 = 其未结订单保证金之和
- [x] 分期不变量（`StreamInvariant.t.sol`）：两份合约余额各自等于所欠；全系统代币守恒（含保证金）；已领取 ≤ 已解锁 ≤ 净本金；争议后解锁额不再增长；StreamEscrow 锁定的保证金 = 其未结订单保证金之和
- [x] 行覆盖率 100%、Slither 无高危
