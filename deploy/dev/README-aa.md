# 本地 AA 开发链

anvil fork Arbitrum Sepolia（EntryPoint v0.7、ZeroDev Kernel v3.3、WebAuthn 验证器都是链上现成的），
部署本仓库的 MockUSDT、EscrowConfig、Escrow、FixedFeePaymaster，再起一个 Pimlico Alto bundler。
钱包前端（`apps/wallet`）和 AA 端到端测试都跑在这条链上。

**仅限开发。** 里面用到的私钥全部由固定字符串派生（`cast keccak "akari-dev-only:<name>"`），公开、无价值。

## 启动 / 停止

```sh
deploy/dev/aa.sh up       # 约 30 秒：anvil → 核对 AA 合约 → 部署 → 充押金 → 写 chain.local.json → Alto
deploy/dev/aa.sh status
deploy/dev/aa.sh down     # 删除两个容器；链状态不保留，下次 up 重新 fork
```

需要 Docker、Foundry（`~/.foundry/bin`）、`jq`。有 HTTP 代理时容器直接用宿主机的 `HTTP(S)_PROXY`
（两个服务都是 `network_mode: host`），本机访问 anvil / bundler 要设 `NO_PROXY=127.0.0.1,localhost`。

| 服务    | 地址                    | 说明                                                                                    |
| ------- | ----------------------- | --------------------------------------------------------------------------------------- |
| anvil   | `http://127.0.0.1:8545` | Foundry v1.8.3 镜像；`--hardfork osaka`（0x100 有 P-256 预编译）；**链 ID 改成 31337**  |
| bundler | `http://127.0.0.1:4337` | `ghcr.io/pimlicolabs/alto:v1.2.8`（按 digest 锁定）；`--safe-mode=false`，开了 CORS     |

可选环境变量：`AA_FORK_URL`（默认 `https://sepolia-rollup.arbitrum.io/rpc`）、`AA_FORK_BLOCK`（默认最新块）、
`AA_ANVIL_PORT`、`AA_BUNDLER_PORT`。

链 ID 改成 31337 是故意的：本地签的报价、代付签名的 EIP-712 域 / 哈希都绑定链 ID，拿到真实 Sepolia 上无法重放。
fork 来的状态（EntryPoint、Kernel 工厂等）不受影响。

## 地址

部署账户是新派生的地址（在 Sepolia 上 nonce 为 0），所以无论 fork 哪个区块，我们的合约地址都相同，
`frontend/packages/contracts/src/chain.local.json` 可以入库，重新 `up` 不会产生改动。

链上现成合约（2026-10-19 在 Arbitrum Sepolia 用 `eth_getCode` 核对，`aa.sh up` 每次在 fork 上再核对一次）：

| 合约                                     | 地址                                         | 代码字节 |
| ---------------------------------------- | -------------------------------------------- | -------- |
| EntryPoint v0.7                          | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` | 16035    |
| Kernel v3.3 实现                         | `0xd6CEDDe84be40893d153Be9d467CD6aD37875b28` | 24469    |
| Kernel v3.3 工厂                         | `0x2577507b78c2008Ff367261CB6285d44ba5eF2E9` | 950      |
| Kernel meta factory（FactoryStaker）     | `0xd703aaE79538628d27099B8c4f621bE4CCd142d5` | 1871     |
| ZeroDev WebAuthn 验证器 0.0.3（patched） | `0x7ab16Ff354AcB328452F1D445b3Ddee9a91e9e69` | 4739     |
| Solady ERC-6492 验证器                   | `0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e` | 64       |
| CREATE2 部署器                           | `0x4e59b44847b379578588920cA78FbF26c0B4956C` | 69       |

本仓库部署的合约：

| 合约              | 地址                                         | 说明                                              |
| ----------------- | -------------------------------------------- | ------------------------------------------------- |
| MockUSDT          | `0x8c313D0eC280837325fA5aB05AC04Be31F46AbBC` | 任何人可 `mint`                                   |
| EscrowConfig      | `0x5b8A58E060327C720Db5FCb1a2BB7C217B292c89` | `contracts/script/Deploy.s.sol`，默认参数         |
| Escrow            | `0xFb3ca740dac6A93a5110a0F8d9cc584ACE80940C` | 同上                                              |
| FixedFeePaymaster | `0x3dd2Fe1Ec5Cfe2EE0ABe62c1e3D81e3d18ee4E1e` | `forge create`；押金 10 ETH、质押 1 ETH（1 天）   |

| 开发账户        | 地址                                         | 用途                                                        |
| --------------- | -------------------------------------------- | ----------------------------------------------------------- |
| deployer        | `0xEE56770A46E04f6323AA23dC22f5935a45d933aE` | 部署；兼任 owner、仲裁、手续费收款与 Paymaster 金库         |
| sponsor         | `0x9A53c3ACF864004F00BdE6E48Bd8A50D6470e8A8` | Paymaster `signer`；前端 DEV 代付用它的私钥签名             |
| bundler 执行者  | `0x19d6319D75788D0fc7f71f3b49fa11863412c804` | Alto `--executor-private-keys`                              |
| bundler 工具户  | `0x685bb2D07a60B63Bd337a0a67b02Ee7749a4Ade3` | Alto `--utility-private-key`                                |

给某个钱包打测试 USDT：

```sh
cast send 0x8c313D0eC280837325fA5aB05AC04Be31F46AbBC 'mint(address,uint256)' <钱包地址> 100000000 \
  --rpc-url http://127.0.0.1:8545 \
  --private-key 0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81   # deployer
```

## 钱包前端 + 端到端测试

```sh
cd frontend
NEXT_PUBLIC_DEV_SPONSOR=1 pnpm --filter @akari/wallet dev   # http://localhost:3001/setup
pnpm e2e:aa                                                 # Playwright + CDP 虚拟 Passkey，链没起来时跳过
```

- `NEXT_PUBLIC_DEV_SPONSOR=1` 让前端用 sponsor 开发私钥直接签 `paymasterAndData`（`lib/wallet/dev-sponsor.ts`）。
  只对 `next dev` 生效：`next build` 带这个变量会直接失败，构建产物里 `akari-dev-sponsor` 解析到不含私钥的空实现，
  `pnpm build` 最后还会扫一遍 `out/` 确认私钥不在里面。生产代付签名只在 forum-sponsor 里做。
- Passkey 的 RP ID 默认 `localhost`（`NEXT_PUBLIC_WALLET_RP_ID`）。
- 钱包页面同源请求 `/api/v1`，`next dev` 转发到 `API_ORIGIN`（默认 `http://127.0.0.1:8080`）；生产构建用
  `NEXT_PUBLIC_API_ORIGIN` 直连论坛 API（ADR 0009）。forum-api 的 `WALLET_ORIGINS` 要包含钱包页面的源。
- 端到端测试按接口逐个选择后端：`WALLET_E2E_API`（默认 `http://127.0.0.1:8080`）上的 forum-api 实现了的接口走真实后端
  （登录与开通验证码经 Mailpit，需要 forum-worker；用户名直接写数据库，`WALLET_E2E_DATABASE` 指定库名），
  没实现的（或 forum-api 没运行）由 `e2e/fake-backend.ts` 模拟；有 `/api/v1/aa/rpc` 时不用 DEV 代付。
- 合约 ABI 改了以后：`pnpm --filter @akari/contracts generate`（先 `forge build`，再用 @wagmi/cli 重新生成 `src/generated.ts`）。
