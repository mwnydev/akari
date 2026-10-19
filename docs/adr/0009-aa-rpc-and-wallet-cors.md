# 0009 钱包提交走标准 ERC-7677 + ERC-4337 JSON-RPC；钱包子域名只加 CORS

- 状态：已采纳（lead 按「安全优先、其次性能」决定，2026-10-19）
- 日期：2026-10-19

## 背景

钱包页面（Cloudflare Pages 上的钱包子域名）要找 forum-sponsor 拿代付签名、提交 UserOperation、查回执；同时要调论坛 API（钱包、报价、订单）。需要定两件事：前后端之间用什么接口，以及钱包子域名怎么带上论坛会话。

## 决定

1. **接口用现成标准，不自造**：forum-api 提供 `POST /api/v1/aa/rpc`（JSON-RPC 2.0），实现
   - ERC-7677：`pm_getPaymasterStubData`、`pm_getPaymasterData`；
   - ERC-4337 bundler 子集：`eth_sendUserOperation`、`eth_estimateUserOperationGas`、`eth_getUserOperationReceipt`、`eth_getUserOperationByHash`、`eth_supportedEntryPoints`、`eth_chainId`；
   - 兼容 `pimlico_getUserOperationGasPrice`（ZeroDev SDK 取 gas 价格用）。

   viem / ZeroDev 原生支持这两个标准，前端只需把 paymaster 和 bundler 地址指向这里；以后换第三方 Bundler 作备用，前端不用改。forum-api 校验会话、Origin（只认钱包子域名）、限流后，经本机 Unix socket 转给 forum-sponsor；RPC 与 Bundler 密钥永远不到浏览器（plan 第 2 节）。
2. **会话不改作用域，只加 CORS**：钱包子域名与论坛同属一个可注册域名，浏览器向论坛主机发的同站请求会带上论坛的会话 Cookie（`SameSite=Lax`，host-only）。所以不把 Cookie 扩到父域名（那样所有子域名都能收到会话），只在 forum-api 加 CORS：仅放行 `WALLET_ORIGINS`、允许凭据。

## 后果

- 论坛主站的页面不能调钱包相关的写接口（Origin 不在钱包列表），与 plan 第 5 节「钱包 Passkey 只认钱包域名」一致。
- 钱包与论坛如果将来分属不同的可注册域名，同站假设失效，届时需要另行设计（例如钱包单独登录），并写新 ADR。
