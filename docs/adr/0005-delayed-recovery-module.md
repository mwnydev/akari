# 0005 恢复码 48 小时延迟恢复：自写最小执行器

- 状态：已采纳（按 plan 第 5 节「不支持则在合约审计范围内补一个最小的延迟恢复模块」）
- 日期：2026-10-19
- 解决：plan 第 17 节待决「ZeroDev 恢复模块能否做到 48 小时延迟 + 可取消」

## 背景

- ZeroDev 官方 `RecoveryAction.doRecovery`（Kalos 审计）：守护者可立即替换验证器，**没有延迟、没有否决**。
- `kernel-7579-plugins` 的 `TimelockPolicy` 能做「提议 → 等待 → 执行」，账户本身可取消；但依赖 EntryPoint v0.8，在 Kernel v3.3 / v0.7 上的兼容性未验证，且**没有审计报告**。
- Rhinestone core-modules（Ackee 审计）的 `SocialRecovery` 没有延迟。

## 决定

自写一个最小的 ERC-7579 执行器模块 `DelayedRecovery`，纳入合约审计：

- 安装时登记恢复地址（恢复码派生，plan 第 5 节）。
- `propose(newSigner)` 只能由恢复地址调用，记录生效时间 = 当前 + 48 小时（常量，不可配置）。
- `cancel()` 只能由账户本身（即原 Passkey 签的 UserOp）调用。
- `execute()` 到期后任何人可调，通过 Kernel 换掉根验证器。
- 每个账户同一时间只有一个提议；事件供 indexer 触发三路提醒。

`TimelockPolicy` 作为备选：若第 6 周前确认它兼容 v0.7 且审计方愿意一并审，可替换本方案。

## 后果

- 审计范围再加一份小合约。和 ADR 0004 一起，第 6 周联系审计机构时要把范围说清楚：`Escrow`、`StreamEscrow`、Paymaster、`DelayedRecovery`。
- 恢复工具（离线）需要支持「部署 + propose」与「execute」两步，上线清单的恢复实测覆盖这条路径。
