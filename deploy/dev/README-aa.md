# 本地 AA 开发链

anvil fork Arbitrum Sepolia（EntryPoint v0.7、ZeroDev Kernel v3.3、WebAuthn 验证器都是链上现成的），
部署本仓库的 MockUSDT、EscrowConfig、Escrow、StreamEscrow、FixedFeePaymaster，再起一个 Pimlico Alto bundler。
钱包前端（`apps/wallet`）、后端（forum-api / forum-indexer / forum-sponsor）和 AA 端到端测试都跑在这条链上。

**仅限开发。** 里面用到的私钥全部由固定字符串派生（`cast keccak "akari-dev-only:<name>"`），公开、无价值。

## 启动 / 停止

```sh
deploy/dev/aa.sh up       # 约 30 秒：anvil → 核对 AA 合约 → 部署 → 充押金 → 核对前端链配置 → 写后端链配置 → Alto
deploy/dev/aa.sh status
deploy/dev/aa.sh down     # 删除两个容器与 chain.local.toml、.env.aa；链状态不保留，下次 up 重新 fork
```

需要 Docker、Foundry（`~/.foundry/bin`）、`jq`。有 HTTP 代理时容器直接用宿主机的 `HTTP(S)_PROXY`
（两个服务都是 `network_mode: host`），本机访问 anvil / bundler 要设 `NO_PROXY=127.0.0.1,localhost`。

| 服务    | 地址                    | 说明                                                                                    |
| ------- | ----------------------- | --------------------------------------------------------------------------------------- |
| anvil   | `http://127.0.0.1:8545` | Foundry v1.8.3 镜像；`--hardfork osaka`（0x100 有 P-256 预编译）；**链 ID 改成 31337**  |
| bundler | `http://127.0.0.1:4337` | `ghcr.io/pimlicolabs/alto:v1.2.8`（按 digest 锁定）；`--safe-mode=false`，开了 CORS     |

两个服务都只监听 127.0.0.1（宿主机网络，局域网不可达）。Alto 把监听地址写死成 `0.0.0.0`，compose 用 Node 预加载脚本
（`alto-loopback`）改成 127.0.0.1；`aa.sh up` 启动后用 `ss` 核对两个端口，监听了别的地址就报错退出。

可选环境变量：

| 变量              | 默认                                     | 说明                                                                  |
| ----------------- | ---------------------------------------- | --------------------------------------------------------------------- |
| `ANVIL_PORT`      | 8545                                     | anvil 端口                                                            |
| `BUNDLER_PORT`    | 4337                                     | Alto 端口                                                             |
| `AA_PROJECT`      | `akari-dev`                              | compose 项目名；同一台机器上多份检出同时运行时各取一个，容器互不覆盖  |
| `AA_SPONSOR_PORT` | 8090                                     | 写进 `.env.aa` 的 forum-sponsor 端口                                  |
| `AA_FORK_URL`     | `https://sepolia-rollup.arbitrum.io/rpc` | fork 来源                                                             |
| `AA_FORK_BLOCK`   | 最新块                                   | fork 区块                                                             |

`down` / `status` 要带上与 `up` 相同的 `ANVIL_PORT`、`BUNDLER_PORT`、`AA_PROJECT`。例如另开一套：

```sh
ANVIL_PORT=8445 BUNDLER_PORT=8446 AA_PROJECT=akari-mine AA_SPONSOR_PORT=8490 deploy/dev/aa.sh up
```

非默认端口只进后端配置（`chain.local.toml`、`.env.aa`）；钱包前端与 e2e 读入库的 `chain.local.json`，仍连默认端口。

链 ID 改成 31337 是故意的：本地签的报价、代付签名的 EIP-712 域 / 哈希都绑定链 ID，拿到真实 Sepolia 上无法重放。
fork 来的状态（EntryPoint、Kernel 工厂等）不受影响。

## 地址

部署账户是新派生的地址（在 Sepolia 上 nonce 为 0），所以无论 fork 哪个区块，我们的合约地址都相同。
`frontend/packages/contracts/src/chain.local.json` 因此是入库的固定文件：只含这些地址与默认端口的 URL，
`aa.sh up` 不改写它，只核对它与本次部署一致（不一致就打印差异并退出）。确实要改（比如部署脚本多了一笔交易、
地址跟着变了）时运行 `AA_WRITE_CHAIN_JSON=1 deploy/dev/aa.sh up`，再提交改动。

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
| StreamEscrow      | `0x6022875397e11A32BbAEB91d80258CBF4AA6F4A8` | 同上                                              |
| FixedFeePaymaster | `0x3dd2Fe1Ec5Cfe2EE0ABe62c1e3D81e3d18ee4E1e` | `forge create`；押金 10 ETH、质押 1 ETH（1 天）   |

| 开发账户        | 地址                                         | 用途                                                        |
| --------------- | -------------------------------------------- | ----------------------------------------------------------- |
| deployer        | `0xEE56770A46E04f6323AA23dC22f5935a45d933aE` | 部署；兼任 owner、仲裁、手续费收款与 Paymaster 金库         |
| sponsor         | `0x9A53c3ACF864004F00BdE6E48Bd8A50D6470e8A8` | Paymaster `signer`；forum-sponsor 与前端 DEV 代付用它签名   |
| relayer         | `0x8204490c73DceDE91340e855279A8A67758609AC` | forum-sponsor 代发热钱包（`aa.sh up` 给它 100 ETH）         |
| bundler 执行者  | `0x19d6319D75788D0fc7f71f3b49fa11863412c804` | Alto `--executor-private-keys`                              |
| bundler 工具户  | `0x685bb2D07a60B63Bd337a0a67b02Ee7749a4Ade3` | Alto `--utility-private-key`                                |

给某个钱包打测试 USDT：

```sh
cast send 0x8c313D0eC280837325fA5aB05AC04Be31F46AbBC 'mint(address,uint256)' <钱包地址> 100000000 \
  --rpc-url http://127.0.0.1:8545 \
  --private-key 0x2bd477e315997bfc5018a2d4ef4780999f011bceb8cf602c009337739f641d81   # deployer
```

## 后端接本地链

`aa.sh up` 生成（都不入库，按检出各自一份）：

- `deploy/dev/chain.local.toml`：后端链配置（`CHAIN_CONFIG`）。合约地址同上表；`start_block` 是部署前的区块
  （fork 的区块号很大，indexer 不能从 0 补拉）；`[aa]` 是 EntryPoint、Paymaster 与 Kernel meta factory；
  `wallet_init_versions` 第 1 版与前端 `localChain` 的版本 1 逐项相同，不同的话钱包页面会拒绝操作。
- `deploy/dev/.env.aa`：`CHAIN_CONFIG`、`SPONSOR_URL`、`SPONSOR_LISTEN`、`SPONSOR_API_TOKEN`、
  `SPONSOR_DEV_PAYMASTER_KEY`（sponsor 开发私钥）、`SPONSOR_DEV_HOT_WALLET_KEY`（relayer 开发私钥）、
  `FALLBACK_BUNDLER_URL`（Alto）。

```sh
set -a; source backend/.env; source deploy/dev/.env.aa; set +a   # .env.aa 在后，覆盖 CHAIN_CONFIG
cd backend
cargo run -p forum-sponsor &   # 代付签名 + 私有代发器，只听 $SPONSOR_LISTEN
cargo run -p forum-indexer &   # 订单状态；anvil 每秒一块，约 1 分钟到 finalized
cargo run -p forum-worker &    # 登录与开通交易的验证码邮件（Mailpit）
cargo run -p forum-api         # /api/v1/aa/rpc 转发给 forum-sponsor
```

forum-api 还需要 `backend/.env.example` 里的其他变量；`WALLET_ORIGINS`、`ALLOWED_ORIGINS` 要包含钱包页面的源。
`.env.aa` 的私钥都由公开字符串派生，forum-sponsor 在主网（链 ID 42161）上拒绝读取环境变量里的私钥。

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
  （登录与开通验证码经 Mailpit，需要 forum-worker；用户名与测试版块 `e2e` 直接写数据库，`WALLET_E2E_DATABASE`
  指定库名），没实现的（或 forum-api 没运行）由 `e2e/fake-backend.ts` 模拟；有 `/api/v1/aa/rpc` 时不用 DEV 代付。
  上面四个后端程序都在跑时就是完整的真实流程：卖家发交易帖并签公开报价 → 买家经 forum-sponsor 代付、代发付款 →
  indexer 把报价标为已付款 → 卖家标记发货 → 买家确认收货 → 等到 finalized。订单页按站内订单 ID 打开，
  链上操作用订单的 `chain_order_id` 与 `contract`。
- 钱包首页（余额、提币地址、提回交易所）与紧急取回页也各有一组测试：`e2e/withdraw.spec.ts`、`e2e/emergency.spec.ts`。
  紧急取回页不连后端（测试里掐断全部 `/api/v1` 请求），经 Alto 不带 Paymaster 提交，ETH 用 `anvil_setBalance` 打进钱包；
  超时取回那条会把链上时间往后拨一天，用 `evm_snapshot` / `evm_revert` 包住，跑完时间复原。
- fork 来的状态是按需从 `AA_FORK_URL` 拉的，公共 RPC 不保留太旧区块的状态：`up` 之后隔得久了，新钱包地址会报
  `historical state … is not available`，`aa.sh down && aa.sh up` 重新 fork 即可。
- 合约 ABI 改了以后：`pnpm --filter @akari/contracts generate`（先 `forge build`，再用 @wagmi/cli 重新生成 `src/generated.ts`）。
