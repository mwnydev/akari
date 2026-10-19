# 0008 钱包初始化参数版本化

- 状态：已采纳（lead 按「安全优先」决定，2026-10-19）
- 日期：2026-10-19

## 背景

Kernel v3.3 账户的初始化参数（`initConfig`）里预设了「给 Paymaster 授权 USDT」（ADR 0004：代付费要在验证阶段扣，必须在第一笔操作前就有授权）。CREATE2 地址由初始化参数决定，所以**钱包地址取决于当时的 Paymaster 地址和 USDT 地址**。

如果以后因为漏洞重新部署 Paymaster，或者换代币，前端按「当前配置」推算地址，就会和老用户的真实地址不一致：显示错余额，甚至让用户往错的地址充值。

## 决定

- 钱包初始化参数（Kernel 实现与工厂、WebAuthn 验证器、Paymaster 地址、授权代币与额度、EntryPoint）组成一个**版本**，写在链配置里，按版本号只增不改。
- `wallets` 表记录每个钱包的 `init_version`；前端推算地址、部署钱包（首笔操作的 `initCode`）一律用该钱包自己的版本，不用「当前」配置。
- 后端绑定钱包地址时校验：按 `init_version` 推算的地址必须与提交的地址一致（`AddressVerifier` 接入链模块后实现）。
- 换 Paymaster 后：新用户用新版本；老用户钱包已部署的，发一笔操作给新 Paymaster 授权；未部署的，首笔操作仍用旧版本的 `initCode` 部署，再授权新 Paymaster。
- 旧版本 Paymaster 合约不删除押金前先确认没有未部署的老钱包依赖它（看 `wallets` 表）。

## 后果

- 链配置多一个 `wallet_init_versions` 列表；前端 `chain.*.json` 同步生成。
- 下一轮实现：后端迁移加 `init_version`，前端 `sdk.ts` 按版本取参数，并加测试「换 Paymaster 后老钱包地址不变」。
