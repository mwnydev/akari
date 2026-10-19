# 开发进度

第 1 周 = 2026-10-19（周一）。计划原文：[plan-v3.md](plan-v3.md) 第 15 节。

## 关键日期

| 里程碑 | 周 | 日期 |
| --- | --- | --- |
| 阶段 0 准备结束 | 3 | 2026-11-06 |
| 最细端到端链路（注册 → 发交易帖 → 开通交易 → 测试网付款 → 发货 → 放款 → indexer → 页面完成） | 5 | 2026-11-20 |
| 合约冻结，开始多方 AI 审计 | 12 | 2027-01-04 |
| 公开审查期（测试网 + 漏洞赏金） | 15–18 | 2027-01-25 – 2027-02-19 |
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

## 第 2–3 周（2026-10-26 – 11-06）

- [x] `Escrow` 核心（一次性模式）：建单（EIP-712 报价 + ERC-6492 / 1271 验签、计次、撤销、最高费率 / gas / 最低手续费保护）、发货、确认、退款、超时、和解、争议、裁决、超时兜底、领取式转账、参数排队 72h、暂停；77 个测试（含模糊与不变量），覆盖率 100%，Slither 无高危
- [x] 后端：迁移（users、forums、topics、posts、post_sources、outbox、只增审计日志等）、邮箱验证码登录、Valkey 会话与滑动窗口限流、Origin 校验、新设备邮件、邮件双通道 + 发件箱 worker、OpenAPI；31 个测试；musl 静态构建验证通过
- [x] 前端：设计系统组件 + 10 个关键页面（模拟数据）+ `/design` 展示页；论坛首屏 JS 174–182 KB
- [x] Ansible 六个角色 + 版本安装 / 回滚脚本；ansible-lint production 档通过，Caddyfile / nftables / systemd 单元实测校验
- [x] CI：合约覆盖率 100% 硬门禁、deploy 检查；后端 CI 带 PostgreSQL + Valkey 服务
- [x] 版本控制改用 Jujutsu（ADR 0006）
- [ ] **设计稿确认（站长）** ← 阶段 0 关卡
- [ ] 下一轮合约：卖家保证金池、`StreamEscrow`、`createFromAuction`、`extendDeadlines`、免费活动排期、代币白名单、部署脚本（含 ERC-6492 验证器检查）
- [ ] 下一轮后端：板块 / 主题 / 楼层 API（楼层分配多 CTE）、ammonia 过滤、apalis 接入、设备管理
- [ ] 下一轮前端：页面接真实 API、Next.js 验证会话 Cookie 签名、Storybook
- [ ] 待补 CI：knip、jscpd、cargo-llvm-cov、lefthook

### 评审记录

- 测试网待实测：USD₮0 代理合约在 Paymaster 验证阶段被读取，第三方备用 Bundler 是否接受（ADR 0004 修订）。
- P1 恢复功能注意：Kernel v3.3 的 Passkey 验证器是共享合约、验证器 ID 不变，恢复时需 `onUninstall` + `onInstall` 原地换钥（同 ZeroDev 已审计的 RecoveryAction）。

- 合约：Offer 原只保护费率和 gas 费，最低手续费上调会多扣旧报价 → 已加 `maxMinFee`。
- 合约：卖家已发货后又主动退款时，平台已垫付的基础 gas 从买家退款里扣（plan「平台不为 gas 倒贴」）。保证金池上线后改从卖家保证金扣，届时复议。
- 前端：金额显示改为至少 2 位小数、去掉尾随 0（`45.00`，`123.456789` 原样）。
- 前端：钱包页首屏 JS 已到 185–200 KB，接入 ZeroDev / viem 时必须懒加载，否则超 200 KB 预算。

## 待决事项跟踪（plan 第 17 节）

| 事项 | 截止 | 负责 | 状态 |
| --- | --- | --- | --- |
| 锁定 EntryPoint 版本 | 第 3 周 | lead | ✅ v0.7 + Kernel v3.3（ADR 0003） |
| 开源 Paymaster 是否支持固定代付费与预扣 | 阶段 2 前 | lead | ✅ 不支持预扣、未审计 → 自写（ADR 0004） |
| ZeroDev 恢复模块 48h 延迟 + 可取消 | 阶段 2 前 | lead | ✅ 官方无延迟 → 自写（ADR 0005） |
| Passkey 中心域名与备用域名 | 第 3 周 | 站长 | 待定 |
| RPC 主备服务商、备用 Bundler | 第 5 周 | 站长 | 待定 |
| 固定 gas 费按测试网校准 | 第 11 周 | lead | 先用 0.1 / 0.02 |
| OKX Arbitrum One 提币费实测；Firefox 关联域名实测 | 上线前 | 站长 | 待定 |

## 偏差与说明

- **不做付费审计**（ADR 0007，站长决定）：改为链上限损（单笔 200、总额 5000 USDT）+ 多方 AI 审计 + Halmos / 变异测试 + 4 周公开审查与赏金。恢复码与 `DelayedRecovery` 推迟到 P1，冻结范围为 `Escrow`、`StreamEscrow`、自写 Paymaster 三份。

- 开发机 Windows 物理内存 31.4 GB，WSL 默认分到约 15.7 GB（50%）。**不调 `.wslconfig`**：日常开发（Rust 编译 + PostgreSQL + Valkey + Next.js）15 GB 够用。计划里「灌 2000 万帖子」那一步（阶段 1 关卡，约第 8 周）改为在生产规格机器上做，或届时临时调到 20 GB 并开 `autoMemoryReclaim=dropCache`，做完改回。
