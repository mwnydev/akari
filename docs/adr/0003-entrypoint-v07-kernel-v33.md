# 0003 EntryPoint v0.7 + Kernel v3.3

- 状态：已采纳
- 日期：2026-10-19
- 解决：plan 第 17 节待决「锁定 EntryPoint 版本」

## 背景

调研（2026-10-19，克隆仓库并在 Arbitrum One 上 `eth_getCode` 核对）：

| 组件 | 支持的 EntryPoint |
| --- | --- |
| ZeroDev Kernel v4.0（2026-10 发布） | 只支持 v0.9 |
| ZeroDev Kernel v3.1–v3.3 | v0.7 |
| Pimlico singleton-paymaster | v0.6 / v0.7 / v0.8，没有 v0.9 |

三个 EntryPoint 在 Arbitrum One 都已部署：v0.7 `0x0000000071727De22E5E9d8BAf0edAc6f37da032`、v0.8 `0x4337084D9E255Ff0702461CF8895CE9E3b5Ff108`、v0.9 `0x433709009B8330FDa32311DF1C2AFA402eD8D009`。Kernel v3.3 工厂与 meta factory 在 Arbitrum One 有代码。

## 决定

- EntryPoint **v0.7**，智能账户 **Kernel v3.3** + ZeroDev WebAuthn 验证器（Kalos 审计）。
- 不用 Kernel v4.0：发布不到一个月，且需要我们把 Paymaster 移植到 v0.9。
- 前端 `@zerodev/sdk` 固定使用 Kernel v3.3 + EntryPoint v0.7 参数；初始参数写入链配置，CREATE2 地址永不改变（plan 第 5 节）。

## 后果

- 以后升到 v0.9 / Kernel v4 意味着新账户地址，旧账户要迁移；上线后不轻易换，需另写 ADR。
- 私有代发器按 v0.7 的 `PackedUserOperation` 与 `handleOps` 实现。
