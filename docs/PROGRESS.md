# 开发进度

第 1 周 = 2026-10-19（周一）。计划原文：[plan-v3.md](plan-v3.md) 第 15 节。

## 关键日期

| 里程碑 | 周 | 日期 |
| --- | --- | --- |
| 阶段 0 准备结束 | 3 | 2026-11-06 |
| 最细端到端链路（注册 → 发交易帖 → 开通交易 → 测试网付款 → 发货 → 放款 → indexer → 页面完成） | 5 | 2026-11-20 |
| **审计机构预约**（关键路径） | 6 | 2026-11-23 |
| 合约冻结送审 | 12 | 2027-01-04 |
| 审计结束 | 17 | 2027-02-12 |
| Arbitrum One 公开上线（单笔 ≤ 1000 USDT） | 22 | 2027-03-15 |
| 拍卖开放 | 24–26 | 2027-03-29 – 2027-04-16 |

## 阶段状态

| 阶段 | 周 | 状态 | 关卡 |
| --- | --- | --- | --- |
| 0 准备 | 1–3 | 进行中 | CI 全绿；设计稿确认 |
| 1 论坛核心 | 3–9 | 未开始 | 2000 万帖子压测达标；PGroonga 定案 |
| 2 担保合约 | 3–11 | 未开始 | 覆盖率 100%、Slither 无高危；第 12 周冻结 |
| 3 交易闭环 | 9–17 | 未开始 | 测试网走通第 11 节全部闭环 |
| 4 加固 | 12–21 | 未开始 | 第 16 节上线清单全部勾选 |
| 5 上线 | 22–26 | 未开始 | 稳定 2–4 周无高危事故 |

## 第 1 周（2026-10-19 – 10-23）

- [x] 本机工具链：Rust 1.99.1 + musl target、Foundry 1.8.3、pnpm 12.11.2、Node 24.22.0
- [x] 仓库顶层结构、计划文档归档、本进度表
- [x] 共享测试向量 `test-vectors/fees.json`（费用、承担方拆分、争议费、分期解锁）
- [x] 后端 Cargo workspace 骨架（4 个程序 + core/db/chain/runtime），core 费用计算过测试向量
- [x] 前端 Turborepo 骨架（forum / wallet / admin + ui / fees / contracts / api-client），fees 过测试向量；typescript-eslint 暂不支持 TS 7，经 `.pnpmfile.cjs` 改用 TS 6 兼容包，支持后删除
- [x] 合约 Foundry 骨架、测试网模拟 USDT（6 位）、FeeMath 过测试向量（覆盖率 100%，Slither 0 发现）
- [x] CI 质量门禁初版（Rust / TS / Solidity + Slither / gitleaks），GitHub Actions 首次全绿（be6881a）；待补：合约覆盖率 100% 硬门禁、knip、jscpd、cargo-llvm-cov、lefthook
- [x] 本地依赖 compose（PostgreSQL 18.6 + PGroonga 4.1.0、Valkey 9.1.2、Mailpit 1.31.6）
- [x] ADR 0001–0002；威胁分析 v0（docs/threat-model.md）
- [x] ADR 0003–0005：EntryPoint v0.7 + Kernel v3.3；自写固定费 Paymaster（验证阶段预扣）；自写 48h 延迟恢复模块

## 第 2–3 周（待拆）

- Ansible 服务器初始化、邮件双通道（Resend + SMTP）
- 设计系统令牌 + 10 个关键页面设计稿（需站长确认）
- 资金与账号威胁分析文档
- 第 3 周起：阶段 1 账号 / 板块，阶段 2 `Escrow` 状态机

## 待决事项跟踪（plan 第 17 节）

| 事项 | 截止 | 负责 | 状态 |
| --- | --- | --- | --- |
| 锁定 EntryPoint 版本 | 第 3 周 | lead | ✅ v0.7 + Kernel v3.3（ADR 0003） |
| 开源 Paymaster 是否支持固定代付费与预扣 | 阶段 2 前 | lead | ✅ 不支持预扣、未审计 → 自写（ADR 0004） |
| ZeroDev 恢复模块 48h 延迟 + 可取消 | 阶段 2 前 | lead | ✅ 官方无延迟 → 自写（ADR 0005） |
| Passkey 中心域名与备用域名 | 第 3 周 | 站长 | 待定 |
| 审计机构与预算 | 第 6 周 | 站长 | 待定 |
| RPC 主备服务商、备用 Bundler | 第 5 周 | 站长 | 待定 |
| 固定 gas 费按测试网校准 | 第 11 周 | lead | 先用 0.1 / 0.02 |
| OKX Arbitrum One 提币费实测；Firefox 关联域名实测 | 上线前 | 站长 | 待定 |

## 偏差与说明

- **合约审计范围扩大**（ADR 0004、0005）：除 `Escrow`、`StreamEscrow` 外，加自写 Paymaster 与 `DelayedRecovery` 两份小合约。第 12 周冻结不变，第 6 周预约审计时按 4 份合约报范围。

- 开发机 Windows 物理内存 31.4 GB，WSL 默认分到约 15.7 GB（50%）。**不调 `.wslconfig`**：日常开发（Rust 编译 + PostgreSQL + Valkey + Next.js）15 GB 够用。计划里「灌 2000 万帖子」那一步（阶段 1 关卡，约第 8 周）改为在生产规格机器上做，或届时临时调到 20 GB 并开 `autoMemoryReclaim=dropCache`，做完改回。
