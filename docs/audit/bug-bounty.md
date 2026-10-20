# 漏洞赏金（草案）

- 状态：**草案，金额与预算待站长确认**（ADR 0007 第 4 条：用手续费收入设小额赏金，最高 500 USDT 起步）
- 适用期：公开审查期（第 15–18 周，2027-01-25 – 2027-02-19）起，主网上线后继续有效；金额随链上上限放开再调
- 冻结版本与文件哈希：见 [freeze-checklist.md](freeze-checklist.md) 第 1 节；赏金只针对该版本（或之后按第 11 节变更流程重新冻结的版本）

## 1. 范围

| 在范围内 | 说明 |
| --- | --- |
| `contracts/src/Escrow.sol` | 一次性担保、统一卖家保证金池、拍卖成交 |
| `contracts/src/StreamEscrow.sol` | 分期担保 |
| `contracts/src/EscrowConfig.sol` | 参数、72 小时排队、暂停、免费活动、代币白名单、在押上限计数 |
| `contracts/src/MarketBase.sol`、`contracts/src/libraries/FeeMath.sol` | 共用的报价计数、领取式转账、费用与解锁计算 |
| `contracts/src/aa/FixedFeePaymaster.sol` | EntryPoint v0.7 固定费 Paymaster |

不在范围内：

- 论坛、后端、forum-sponsor、钱包前端、后台（另有渗透测试，plan 第 16 节）；钓鱼、社工、DoS 服务器
- 第三方组件本身：USD₮0（含发行方冻结地址）、EntryPoint v0.7、ZeroDev Kernel v3.3、Safe、OpenZeppelin、Solady、Arbitrum 排序器
- `MockUSDT.sol`（只在测试网）与 `test/`、`script/` 下的代码
- [freeze-checklist.md](freeze-checklist.md) 第 8 节「已知限制与接受的风险」里已列出的问题，以及需要以下前提的攻击：金库 / 仲裁 / 运营 Safe 的 2/3 签名人被攻破、forum-sponsor 私钥被盗后只造成该节已写明的后果（如耗尽 Paymaster 押金、拍卖以 ≥ 保留价卖给他人）
- 仲裁 Safe 按规则执行的裁决本身是否公正（属于治理问题，不是合约漏洞）
- 只影响 gas 优化、代码风格、事件字段的建议（欢迎提 issue，但不发赏金）

## 2. 等级与金额（提案）

链上限损：单笔 ≤ 200 USDT、全站在押 ≤ 5000 USDT（合约强制，ADR 0007）。最坏情况下合约里最多 5000 USDT，赏金按「可能造成的损失」定，封顶 500 USDT（≈ 全站上限的 10%），让上报比利用更划算，同时不超出手续费收入能承担的范围。

| 等级 | 例子 | 金额（USDT） |
| --- | --- | --- |
| 严重 | 转走或永久锁死**不属于自己的**托管资金或保证金（多笔订单或全站）；绕过全站在押上限或单笔上限建单；把订单资金付给买家、卖家、收款地址以外的地址；Paymaster 从用户账户拉走超过签名费用或超过 1 USDT | 500 |
| 高 | 单笔订单资金被盗或永久锁死（没有任何超时 / 仲裁出口）；同一场拍卖成交两次或以低于保留价、低于签名汇率成交；手续费、gas 费超过卖家报价里签的上限；绕过 72 小时排队提高收费 | 200 |
| 中 | 资金能取回但被恶意延迟超过合约承诺的期限；按比例结算的金额偏差超过 1 个最小单位并可被反复利用；不靠私钥就能耗尽 Paymaster 押金；让暂停失效或让暂停影响已有订单的出口 | 50–100 |
| 低 | 与规格（plan 第 6 节、`contracts/src/README.md`）不一致但不造成损失；超出 freeze-checklist 第 8 节 L10 所述范围的舍入偏差；可让索引器误读的事件错误 | 20 |

- 同一根因的多个报告只发一份，给最先提交且附可运行 PoC 的人
- 预算提案：每个公开审查期 1500 USDT 封顶，从金库手续费收入支付；用尽后当期剩余报告记入致谢、下期补发
- 发放：Arbitrum One 上的 USDT，由金库 Safe 支付；发放记录写进 `docs/audit/`

## 3. 规则

1. 只能在本地（`anvil`）、Arbitrum Sepolia 测试网部署或主网分叉上验证；**禁止在主网实际利用**，包括「先转走再归还」
2. 报告必须附 PoC：能在本仓库 `contracts/` 下 `forge test` 运行、在修复前失败的 Foundry 测试（格式见 [ai-audit-pack.md](ai-audit-pack.md) 第 6 节）；纯理论描述最多按「低」处理
3. 公开披露前给我们 30 天修复时间；修复并重新部署（或确认不修）后可以公开，我们会在 `docs/audit/` 致谢（可匿名）
4. 不得访问其他用户的数据、不得对服务器做压力测试、不得冒充平台人员
5. 站长有权按影响重新定级；有异议可要求第二位维护者复核一次
6. 为了领取赏金需要提供一个 Arbitrum One 收款地址；不要求实名，但受制裁地区与制裁名单上的地址不能领取

## 4. 上报与处理流程

| 步骤 | 时限 |
| --- | --- |
| 私下提交：GitHub 仓库的 Private vulnerability reporting（Security → Report a vulnerability），或邮件 `security@akari.cc`（**待开通**，开通后在此确认）。不要开公开 issue | — |
| 确认收到 | 2 个工作日内 |
| 复现并定级（把 PoC 加进测试套件，确认修复前失败） | 7 天内 |
| 修复：严重 / 高 → 金库 Safe 先 `setPaused(true)` 停止新订单（已有订单不受影响），部署修复后的新合约，旧订单在旧合约里走完（plan 第 13 节合约迁移流程）；中 / 低 → 下一次冻结版本 | 严重 / 高 立即；中 / 低 按变更流程 |
| 修复版本重新跑全部门禁（[freeze-checklist.md](freeze-checklist.md) 第 10 节）并发放赏金 | 修复后 14 天内 |

## 5. 测试网地址（待部署）

| 合约 | Arbitrum Sepolia 地址 |
| --- | --- |
| MockUSDT（测试用，任何人可 mint） | `TBD` |
| EscrowConfig | `TBD` |
| Escrow | `TBD` |
| StreamEscrow | `TBD` |
| FixedFeePaymaster | `TBD` |
| EntryPoint v0.7 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |

部署后填入地址与区块号，并在 Arbiscan（Sepolia）验证源码；参数与主网相同（[freeze-checklist.md](freeze-checklist.md) 第 9 节），仲裁、金库 Safe 在测试网用测试 Safe。
