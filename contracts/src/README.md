# 合约规格清单（plan-v3 第 6 节）

两份不可升级合约 `Escrow`（一次性）与 `StreamEscrow`（分期），共用 `FeeMath`、签名与权限代码，一起审计。第 12 周（2027-01-04）冻结送审。

## 已完成

- [x] `libraries/FeeMath.sol`：手续费（含活动费率）、承担方拆分、买家应付、争议费、分期净本金 / 解锁 / 领取净额；过 `test-vectors/fees.json`
- [x] `MockUSDT.sol`：仅测试网，6 位小数

## 状态机

`createAndFund` → `Funded` →（`Disputed`）→ `Released` | `Refunded` | `Settled`。没有「已创建未付款」状态。

## 函数（两份合约通用，除注明外）

- [ ] `createAndFund(offer, sellerSig)` 买家：EIP-712 + ERC-6492 验卖家报价，按实际到账余额记账，同交易锁定卖家保证金
- [ ] `markDelivered`（Escrow）/ `start`（StreamEscrow）卖家
- [ ] `release` 买家（Escrow）
- [ ] `claim` 卖家（StreamEscrow）：可领 > 0.02 USDT，间隔 ≥ 24h，当次扣领取 gas 费
- [ ] `refund` 卖家主动全额退款（不收手续费）
- [ ] `proposeEnd` / `acceptEnd` 协商和解，72h 失效
- [ ] `dispute` 发起方预付争议费；卖家可用已解锁未领余额或可用保证金抵扣；StreamEscrow 记录 `disputedAt`
- [ ] `resolve(orderId, toBuyerBps, bondToBuyerBps)` 仅仲裁 Safe
- [ ] `claimTimeout` 任何人：确认期满放款 / 未按期发货退款 / 分期未开始退款
- [ ] `resolveTimeout` 任何人：争议 30 天未裁决按默认规则
- [ ] `withdraw(to)` 领取式转账余额
- [ ] `cancelOffer(nonce)` 卖家
- [ ] `extendConfirm(orderId, secs)` 卖家（累计上限）；买家最后 6h 内免费紧急顺延 1 次 24h
- [ ] `createFromAuction(授权, 卖家签名, 成交证明, 平台签名)` 拍卖赢家；每场只成交一次、成交价 ≥ 保留价
- [ ] `extendDeadlines(orderIds, secs)` 仲裁 Safe：单次 ≤ 7 天、累计 ≤ 14 天
- [ ] `depositBond` / `withdrawBond`（Escrow 统一保证金池，授权 StreamEscrow 锁定 / 解锁 / 赔付）
- [ ] `pause` 只阻止新建订单

## 参数（owner = 金库 Safe）

- [ ] 参数排队：收紧 72h 生效、可撤销；放宽立即生效；所有参数建单时写入订单快照
- [ ] 硬上限：费率 ≤ 10%、最低手续费 ≤ 5 USDT、最低订单 1–100 USDT、单笔上限 ≤ 100000 USDT、gas 费 ≤ 1 USDT、争议费倍数 0.5–5、超时兜底 14–60 天、gas 补贴 ≤ 10000 单
- [ ] 免费活动：开始 / 结束 / 活动费率 / 范围，单次 ≤ 90 天；gas 补贴按订单数计数
- [ ] 代币白名单：上线只开 USD₮0，新增经金库 Safe 排队 72h
- [ ] 报价最高费率 / 最高 gas 费保护

## 安全要求

- [ ] OpenZeppelin `SafeERC20` / `ReentrancyGuard` / `EIP712` / `SignatureChecker` + Solady ERC-6492；先改状态再转账
- [ ] 签名域绑定 chainId + 合约地址；按报价哈希计数成交次数
- [ ] 无管理员提款、无代理升级、无 `selfdestruct`
- [ ] 不变量：已领取 + 已退款 + 费用 + 合约余额 = 订单总额
- [ ] 行覆盖率 100%、Slither 无高危
