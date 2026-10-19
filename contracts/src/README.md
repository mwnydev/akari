# 合约规格清单（plan-v3 第 6 节）

两份不可升级合约 `Escrow`（一次性）与 `StreamEscrow`（分期），共用 `FeeMath`、签名与权限代码，一起审计。第 12 周（2027-01-04）冻结送审。

## 已完成

- [x] `libraries/FeeMath.sol`：手续费（含活动费率）、承担方拆分、买家应付、争议费、分期净本金 / 解锁 / 领取净额；过 `test-vectors/fees.json`
- [x] `MockUSDT.sol`：仅测试网，6 位小数

## 合约结构（第 3 轮起）

- `EscrowConfig`：金库 Safe 拥有的全部可调设置，`Escrow` 与以后的 `StreamEscrow` 共用（参数与 72h 排队、暂停、免费活动、代币白名单、拍卖签名人、市场列表、全局在押资金上限计数）。建单时经 `checkout` 取快照。
- `Escrow`：一次性担保 + 统一卖家保证金池；没有 owner。所有可调项都在 `EscrowConfig`，`Escrow` 不可升级、无管理员函数。
- 拆分原因：功能全部放进一份合约会超过 24 KB 代码上限；拆分后 Escrow 21.3 KB、EscrowConfig 11 KB。

## 未做（下一轮）

- [ ] `StreamEscrow`（经 `EscrowConfig.queueMarket` 登记后可调用 `Escrow.lockBond` / `releaseBond`）
- [ ] StreamEscrow 卖家用保证金抵扣争议费（需要 Escrow 增加「从保证金划转给市场」的接口）

## 状态机

`createAndFund` → `Funded` →（`Disputed`）→ `Released` | `Refunded` | `Settled`。没有「已创建未付款」状态。

## 函数（两份合约通用，除注明外）

- [x] `createAndFund(offer, sellerSig)` 买家：EIP-712 + ERC-6492 验卖家报价，按实际到账余额记账，同交易锁定卖家保证金
- [x] `markDelivered`（Escrow）/ `start`（StreamEscrow）卖家
- [x] `release` 买家（Escrow）
- [ ] `claim` 卖家（StreamEscrow）：可领 > 0.02 USDT，间隔 ≥ 24h，当次扣领取 gas 费
- [x] `refund` 卖家主动全额退款（不收手续费）
- [x] `proposeEnd` / `acceptEnd` 协商和解，72h 失效
- [x] `dispute` 发起方预付争议费；卖家可用已解锁未领余额或可用保证金抵扣；StreamEscrow 记录 `disputedAt`
- [x] `resolve(orderId, toBuyerBps, bondToBuyerBps)` 仅仲裁 Safe
- [x] `claimTimeout` 任何人：确认期满放款 / 未按期发货退款 / 分期未开始退款
- [x] `resolveTimeout` 任何人：争议 30 天未裁决按默认规则
- [x] `withdraw(token, to)` 领取式转账余额
- [x] `cancelOffer(nonce)` 卖家
- [x] `extendConfirm(orderId, secs)` 卖家（累计上限）；买家最后 6h 内免费紧急顺延 1 次 24h
- [x] `createFromAuction(授权, 卖家签名, 成交证明, 平台签名)` 拍卖赢家；每场只成交一次、成交价 ≥ 保留价（成交证明带 `authHash` 绑定卖家授权）
- [x] `extendDeadlines(orderIds, secs)` 仲裁 Safe：单次 ≤ 7 天、累计 ≤ 14 天
- [x] `depositBond` / `withdrawBond`（Escrow 统一保证金池，市场经 `lockBond` / `releaseBond` 锁定 / 解锁 / 赔付）
- [x] `pause` 只阻止新建订单

## 参数（owner = 金库 Safe）

- [x] 参数排队：收紧 72h 生效、可撤销；放宽立即生效；所有参数建单时写入订单快照
- [x] 硬上限：费率 ≤ 10%、最低手续费 ≤ 5 USDT、最低订单 1–100 USDT、单笔上限 ≤ 100000 USDT、gas 费 ≤ 1 USDT、争议费倍数 0.5–5、超时兜底 14–60 天、gas 补贴 ≤ 10000 单
- [x] 免费活动：开始 / 结束 / 活动费率 / 范围，单次 ≤ 90 天；gas 补贴按订单数计数
- [x] 代币白名单：上线只开 USD₮0，新增经金库 Safe 排队 72h（只收 6 位小数代币）
- [x] 报价最高费率 / 最高 gas 费 / 最高最低手续费保护
- [x] 全局在押资金上限 `totalLockedCap`（默认 5000 USDT，硬上限 100 万）：在押 = 未结订单持有 + 卖家保证金；新订单与保证金存入超限即拒绝，争议费不受限；单笔上限默认 200 USDT。两个上限调高调低都立即生效（只影响新订单）

## 部署

- `script/Deploy.s.sol`：先部署 `EscrowConfig`（预测 Escrow 地址登记为市场），再部署 `Escrow`；测试网缺 Solady ERC-6492 验证器时经 CREATE2 部署，Arbitrum One 上缺失则中止（需单独、有意识地部署）。
- `foundry.toml` 已配 `arbitrum_sepolia` / `arbitrum` 的 RPC 与 Arbiscan（从环境变量读取）。

## 安全要求

- [x] OpenZeppelin `SafeERC20` / `ReentrancyGuard` / `EIP712` / `SignatureChecker` + Solady ERC-6492；先改状态再转账
- [x] 签名域绑定 chainId + 合约地址；按报价哈希计数成交次数
- [x] 无管理员提款、无代理升级、无 `selfdestruct`
- [x] 不变量：已领取 + 已退款 + 费用 + 合约余额 = 订单总额；合约余额 = 未结订单 + 保证金 + 待领余额；`totalLocked` = 未结订单 + 保证金；每个卖家已锁定保证金 = 其未结订单保证金之和
- [x] 行覆盖率 100%、Slither 无高危
