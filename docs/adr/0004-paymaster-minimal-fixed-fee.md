# 0004 自写极简固定费 Paymaster（验证阶段预扣）

- 状态：已采纳（按 plan 第 3 节「不支持则写极简固定费 Paymaster 并纳入合约审计」）
- 日期：2026-10-19
- 解决：plan 第 17 节待决「开源 Paymaster 是否支持固定代付费与预扣」

## 背景

Pimlico singleton-paymaster（V7）调研结论：

- ERC-20 模式按 `汇率 × gas + constantFee` 收费。把 `exchangeRate` 设为 1 可以近似成固定费，但这时 `preFundInToken` 必须 ≤ 按汇率算出的费用（≈ 0），否则报 `PreFundTooHigh`，**做不了预扣**。
- 不预扣时只在 `postOp` 里 `transferFrom`：业务 `callData` 回滚也照样收（满足要求）；但如果 `callData` 成功地转空余额或撤销授权，`postOp` 转账失败，**gas 由 Paymaster 白付**。这正是 plan 第 5 节要堵的「刷失败交易零成本」。
- 仓库里没有审计报告，搜索也找不到。不管用不用它，都得自己审计。

## 决定

自写一个约 100–150 行的 v0.7 Paymaster，纳入第 12 周冻结的合约审计范围：

- `validatePaymasterUserOp`：校验 forum-sponsor 的签名（绑定 userOpHash、validUntil / validAfter、费用档位）；签名里的费用 > 0 时，**当场** `transferFrom(sender → 金库, fee)`。验证阶段的状态变更不随 `callData` 回滚，用户也没法在 `callData` 里先把钱转走。
- 费用档位只有两种：付款前的操作收代付费（默认 0.05 USDT，合约写死硬上限 1 USDT）；付款后的操作费用为 0（由担保合约结算时扣固定 gas 费）。
- 不做汇率换算，不做 `postOp`（返回空 context），代码面最小。
- 授权来源：钱包部署 `initCode` 里预设的 USDT 授权（plan 第 5 节）。
- 押金提取权在运营 Safe；签名者可由运营 Safe 轮换。
- 依赖私有代发器：验证阶段读写发送者的 USDT 余额属于「关联存储」，Paymaster 质押后公共 Bundler 也允许；备用第三方 Bundler 上线前实测。

## 后果

- 合约审计范围增加一份小合约，必须在第 12 周冻结前完成并测试（单元、模糊、代付失败场景）。
- 在 `test-vectors` 之外补一组 Paymaster 场景测试：业务回滚仍收费、余额不足时验证失败不上链、过期签名拒绝。
