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
- [x] 第 3 轮合约：卖家保证金池、`extendDeadlines`、免费活动排期、代币白名单、`createFromAuction`、全站托管总额上限 5000 / 单笔 200 USDT、部署脚本；为控制合约大小拆出 `EscrowConfig`（Escrow 本身无 owner）
- [x] 第 3 轮合约：自写固定费 Paymaster（EntryPoint v0.7），签名有效期链上强制 ≤ 10 分钟；合约合计 151 个测试，覆盖率 100%，Slither 无高危
- [x] 第 3 轮后端：板块 / 主题 / 楼层 API、楼层分配单条语句、板块列表 ZSET + 深分页游标、首页缓存版本化失效、浏览数批量回写、版主操作写审计日志、新用户限制；58 个测试，所有新 SQL 实测走索引
- [x] 第 3 轮前端：OpenAPI 生成客户端、邮箱登录接真实 API、会话 Cookie 验签（Next.js 只持有签名半密钥）、Playwright 端到端；论坛首屏 143–178 KB
- [x] 第 4 轮合约：`StreamEscrow` 分期担保（共用 `EscrowConfig` 与保证金池，抽出 `MarketBase` 共用签名与领取式转账）；合约合计 223 个测试，覆盖率 100%，Slither 无高危，三份合约都在 24 KB 内
- [x] 第 4 轮后端：钱包 Passkey 注册与再验证（首个 Passkey 需专用「开通交易」邮箱码）、钱包地址绑定、交易帖、报价卡片（EIP-712 哈希与 Solidity 向量一致）；89 个测试；musl 静态构建含 vendored OpenSSL 验证通过
- [x] 第 4 轮前端：钱包 Passkey 智能账户（Kernel v3.3）、卖家签报价（ERC-6492）、买家一次 Face ID 付款、发货、放款；本地 Anvil（分叉 Arbitrum Sepolia）+ Alto Bundler，端到端全程通过；钱包 SDK 懒加载，钱包页首屏 148–166 KB
- [x] 第 4 轮后端：链模块（alloy 绑定由合约 ABI 生成、ABI 过期即测试失败）+ indexer：WebSocket 即时 + 每 5 秒批量校对、区块回滚撤销重放、latest/safe/finalized 分级、RPC 主备切换、单写入者锁；约 2400 次 RPC/小时（逐块轮询 ≥ 14400）；后端合计 123 个测试，含 Anvil 真实部署集成测试
- [x] 第 5 轮：forum-sponsor（ERC-7677 + ERC-4337 JSON-RPC，白名单、费用档位、次数上限、签前 `eth_simulateV1` 模拟、私有代发器 150 ms 合并批量、卡单提价重发、故障切第三方 Bundler、余额告警）
- [x] 第 5 轮：订单接口、分期订单进 indexer、报价自动标记已付款、钱包初始化参数版本化（ADR 0008，服务端链上核验地址）、钱包子域名专用 CORS（ADR 0009）
- [x] 第 5 轮：论坛页面接真实接口（板块、帖子、回复、编辑历史、版主操作、Tiptap 懒加载、交易帖与报价卡片）；钱包前端接真实接口（开通交易、发报价、付款、实时订单页）
- [x] **第 5 周端到端链路打通（提前完成）**：邮箱登录 → 卖家开通交易（Passkey）→ 发交易帖并签报价 → 买家开通交易 → 一次 Face ID 付款（真实代付 + 私有代发器）→ indexer 标记已付 → 发货 → 放款 → 订单页显示最终确认；链上余额核对一致。合约 223 个测试、后端 182 个、前端 21 项任务全部通过
- [x] 第 6 轮：提币地址（首个立即生效、之后 24 小时冷却，EIP-55 校验）、只读链上代理与余额接口、代付只转已生效提币地址；代付提币额度（ADR 0010，签名过期后才退额度与配额）
- [x] 第 6 轮：私信（报价卡片入会话）、订单聊天哈希链、站内通知（按月分区）、WebSocket 推送（跨实例）、在线状态
- [x] 第 6 轮：个人主页、用户名、@搜索、用户名片缓存（等级与信誉）、交易成功率（只认 finalized，防刷）、经验值、跨板块最新主题、图片上传（直传 + worker 转 WebP 清 EXIF）
- [x] 第 6 轮：钱包提回交易所（「全部提回」= 余额 − 代付费）、充值二维码、分期订单开始 / 领取、紧急取回页（无后端可用）；真实端到端 5 个场景通过；后端 257、合约 223、前端 21 项全部通过；musl 静态构建通过
- [x] 第 7 轮：拍卖人民币计价（ADR 0011）：拍卖授权加 `currency` / `cnyPerUsdt`（汇率硬范围 5.0000–10.0000），人民币以分为单位、链上向上取整换算 USDT，三端共用向量；交易帖可选参考汇率。Escrow 22,509 B
- [x] 第 7 轮：管理角色收为站长 / 管理员 / 版主（ADR 0013），统一权限模块、处罚只能向下、版主限本板块；管理操作限速与异常自动暂停；Telegram 管理员群告警；站长批量撤销（可预览）；站长后台（`administrator.akari.cc` Passkey，闲置 2h / 最长 12h，敏感操作 5 分钟内再验证）
- [x] 第 7 轮：仲裁后台：争议自动建案（随回滚撤销 / 重开）、随机分配 + 回避、≥ 200 USDT 双人背靠背、不一致站长终裁、72h 申诉、裁决 Passkey（RP `akari.cc`，挑战绑定案件与分配比例）、哈希链校验失败拒绝裁决、Safe 批量交易文件、执行结果核对
- [x] 第 7 轮：正式域名写死为代码常量（`AKARI_ENV=production` 时环境变量不能改），Cookie 不带 Domain 的测试；主网部署脚本检查仲裁 / 金库 / 合约管理员必须是 2/3 Safe；Pub/Sub 频道前缀；会话活跃时每小时顺延、最长 90 天
- [x] 第 7 轮：源站防护：网站与 SSH 都走 Cloudflare Tunnel，无入站端口；出站端口白名单 + CI 检查新增外部请求；SMTP 仅开发用，生产 Resend + Postmark（HTTP API，不泄露源站 IP）；外网暴露检测脚本；换 IP 手册
- [x] 第 7 轮：论坛前端接上私信、订单聊天、通知、实时推送（跨标签页共用连接）、个人主页、用户名、@提及、图片上传；后端 328、合约 245、前端 21 项全部通过；钱包 e2e 5/5、论坛 e2e 24/24；musl 静态构建通过
- [ ] 下一轮：站长后台前端、论坛仲裁页面、版主管理入口；交易帖「发起担保」按钮；Telegram 告警接入更多事件
- [ ] 下一轮前端：板块 / 帖子页接真实 API、Tiptap 编辑器、游客页 Valkey 缓存、Turnstile
- [ ] 待补 CI：knip、jscpd、cargo-llvm-cov、lefthook

### 评审记录

- 待办：Arbitrum Nitro 的 `eth_simulateV1` 有已知崩溃问题（OffchainLabs/nitro#3372），forum-sponsor 签前模拟依赖它 → 在 Alchemy / Infura 的 Arbitrum Sepolia 上实测；不可靠则降级为 EntryPoint `simulateHandleOp` / `eth_call` 模拟，模拟失败一律拒签（不放行）。
- 第 7 轮：被封号用户不能再发报价、接受报价（会开新订单）、发交易帖；已有订单、订单聊天、提币照常，保证资金不被卡住。
- 第 7 轮：Next.js 只读 `COOKIE_SIGNING_KEY`（恰好 32 字节），给完整密钥会拒绝启动；Ansible 检查 web.env 不含 `COOKIE_SECRET`。
- 第 7 轮：Cloudflare 回源直连方案下，Caddy 在拒绝无客户端证书的连接前已发出带 akari.cc 的证书，会把 IP 和域名关联起来 → 默认改走 Tunnel，直连方案只作应急。
- 第 7 轮：sqlx::test 按测试路径给临时库命名、整个 Postgres 服务器共用，并行检出会互删 → 每个代理用自己的 Postgres 容器。
- 待办：本地 `aa.sh` 会改写受版本控制的 `chain.local.json`；Alto 监听 0.0.0.0（局域网可达），改为 127.0.0.1；e2e README 补 S3 CORS 与 `general` 板块初始化；仲裁与钱包的挑战存取函数重复，合并。

- 第 6 轮：钱包 Passkey 改为可发现凭据（resident key），否则无后端时紧急取回页找不到凭据。
- 第 6 轮：libwebp 未升级——`webp 0.3.1` 只支持 `libwebp-sys 0.9.x`（libwebp 1.3.1，已含 CVE-2023-4863 修复）；换 1.6 需自写 FFI，与 `unsafe_code = "forbid"` 冲突。解码走纯 Rust，libwebp 只编码已解码像素，暴露面低；`webp` 出新版时升级。
- 待办：主网部署脚本必须检查 `arbiter` 是 Safe 合约（有代码、owners/threshold 符合 ADR 0013），不能是个人 EOA。
- 待办：从 Playwright 请求上下文调 `PUT /me/username` 后浏览器会话掉线（curl 正常），原因未查。
- 待办：Valkey Pub/Sub 频道不按数据库隔离，多个检出同时运行会串话（只影响本地开发），频道名加环境前缀。
- 待办：`forum_flow::topic_reply_and_board_order` 偶发 `RESEND_TOO_SOON`。

- 第 5 轮：Escrow 与 StreamEscrow 的链上订单号各自从 1 开始，不唯一 → 订单页一律用数据库 ID，链上调用用 `chain_order_id` + 合约地址。
- 第 5 轮：钱包初始化参数以前端随包发布的配置为准，服务器下发的列表只做交叉核对（服务器被攻破也换不了 Paymaster 或授权字段）。
- 第 5 轮：并行子代理共用临时目录导致一个代理误杀另一个代理的服务 → 以后运行文件只放各自工作区、端口和数据库独立。
- 第 5 轮：OpenAPI 的 operationId 重名会让前端类型串在一起，已改名并加测试防回归。

- 第 4 轮合并：CI 的 sqlx 离线元数据检查失败——元数据须在全新数据库上生成（与 CI 一致），已统一重新生成。indexer 的 ABI 副本在 StreamEscrow 改动 Escrow 后过期，ABI 一致性测试抓到；新事件 `BondTaken` 由穷尽匹配在编译期发现并补上。
- 待清理：后端同时用 `sha3`（报价哈希）和 alloy 自带的 keccak，下一轮统一到 alloy-primitives，少一个依赖。

- 第 4 轮：钱包地址依赖 Paymaster 与 USDT 地址（初始化授权）→ ADR 0008 初始化参数版本化，换 Paymaster 不影响老用户地址。
- 第 4 轮：`takeBond` 只有登记过的市场合约能调（登记需 72 小时排队、不可移除），用于卖家用保证金付分期订单的争议费。
- 第 4 轮：开发用代付私钥曾被打进生产包（Turbopack 打包了不会执行的分支），已改为构建时替换为空桩 + 生产构建扫描产物，发现即失败。
- 第 4 轮：钱包子域名与论坛不同源时 Lax 会话 Cookie 跨站不发送，需在接线时定 Cookie 作用域与 CORS（只放行钱包子域名）。
- 第 4 轮：本地测试链 chain id 用 31337，本地签名不能在真实 Sepolia 重放。

- 第 3 轮：ammonia 4.2.3 有安全公告 RUSTSEC-2026-0373，按 48 小时安全补丁规则升到 4.2.4（我们不允许 `style`，原本也无法利用）。
- 第 3 轮：开发库 `random_page_cost` 改为 1.1 与生产一致，否则规划器在默认值下会选错 ZSET 重建方案。

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
| Passkey 中心域名与备用域名 | 第 3 周 | 站长 | ✅ 钱包 `wallet.akari.cc`、后台 `administrator.akari.cc`（ADR 0012）、论坛主站 `akari.cc`；备用域名待定 |
| RPC 主备服务商、备用 Bundler | 第 5 周 | 站长 | ✅ 主 Alchemy、备 Infura（免费套餐起步，均支持 Arbitrum One / Sepolia 与 WebSocket；用量接近上限或上线后转付费）；密钥站长自行放服务器加密配置。备用 Bundler 待定 |
| 固定 gas 费按测试网校准 | 第 11 周 | lead | 先用 0.1 / 0.02 |
| OKX Arbitrum One 提币费实测；Firefox 关联域名实测 | 上线前 | 站长 | 待定 |

## 偏差与说明

- **不做付费审计**（ADR 0007，站长决定）：改为链上限损（单笔 200、总额 5000 USDT）+ 多方 AI 审计 + Halmos / 变异测试 + 4 周公开审查与赏金。恢复码与 `DelayedRecovery` 推迟到 P1，冻结范围为 `Escrow`、`StreamEscrow`、自写 Paymaster 三份。

- 开发机 Windows 物理内存 31.4 GB，WSL 默认分到约 15.7 GB（50%）。**不调 `.wslconfig`**：日常开发（Rust 编译 + PostgreSQL + Valkey + Next.js）15 GB 够用。计划里「灌 2000 万帖子」那一步（阶段 1 关卡，约第 8 周）改为在生产规格机器上做，或届时临时调到 20 GB 并开 `autoMemoryReclaim=dropCache`，做完改回。
