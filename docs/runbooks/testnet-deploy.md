# Arbitrum Sepolia 测试网部署

把 EscrowConfig、Escrow、StreamEscrow、FixedFeePaymaster（与测试代币 MockUSDT）部署到 Arbitrum Sepolia，配好 Paymaster，
再让测试服务器上的 forum-api / worker / indexer / sponsor 接上这条链（plan 第 13、16 节）。EntryPoint v0.7、Kernel v3.3、
WebAuthn 验证器、Safe 都是链上现成合约（ADR 0003）。

工具都在 `deploy/testnet/`：

| 文件 | 用途 |
| --- | --- |
| `testnet.sh` | 唯一入口：`preflight` → `deploy` → `configure` → `verify` → `emit-config`，每步可重复运行 |
| `.env.example` | 配置模板（只有地址与文件路径，没有密钥），复制成 `deploy/testnet/.env` |
| `rehearse.sh` | 本机 anvil fork 演练整套流程（含四个后端程序与冒烟检查），见文末 |
| `rpc-meter.py` | 本机 JSON-RPC 计数转发，统计 RPC 服务商额度用量 |
| `deployments/arbitrum-sepolia/` | `deploy` 之后生成并入库：部署记录、forge 广播、Safe 交易文件、后端链配置 |

## 已定的取舍

- **代币用 MockUSDT**（`contracts/src/MockUSDT.sol`，6 位小数，任何人可 `mint`）。Arbitrum Sepolia 上没有 USD₮0，
  Circle 的测试 USDC 要从水龙头少量领取、也不是 USDT；测试网要的是随时给测试钱包发币。交易所充值 / 提回的实测本来就只能在
  主网做（plan 第 16 节）。要用别的 6 位小数测试代币时填 `TOKEN`，`preflight` 核对它有代码、`decimals() == 6`。
- **三个 Safe**（与主网相同的分工，测试网上也照真实安排建，演练的就是签名人流程）：

  | Safe | 合约里的角色 | 签名人（门槛） |
  | --- | --- | --- |
  | 仲裁 Safe | Escrow / StreamEscrow 的 `arbiter`（`resolve`、`extendDeadlines`） | 2 名管理员 + 站长，任意 2 人（ADR 0013） |
  | 金库 Safe | EscrowConfig 的 `owner`（改参数，72 小时排队）、两个市场的 `feeRecipient`、Paymaster 的 `treasury` | 站长的 3 个签名设备，任意 2 个（plan 第 9 节） |
  | 运营 Safe | FixedFeePaymaster 的 `owner`（质押、换代付签名者、取回押金） | 站长单独执行（门槛 1；可再加金库设备作签名人以便取回） |

  plan 里的「合约 owner Safe」就是金库 Safe，不另建。部署账户不兼任任何角色，部署完只剩一点测试网 ETH。
- **forum-sponsor 的三把热密钥**只把地址交给部署：`PAYMASTER_SIGNER`（Paymaster 构造时写入）、`RELAYER_ADDRESS`
  （`configure` 给它打 ETH）、`AUCTION_SIGNER`（EscrowConfig 构造时写入；留空 = 拍卖关闭）。私钥只进服务器的 systemd 凭据。
- **RPC 一家也能跑**：主节点必填，备用节点可选（plan 第 2 节要求每类至少两家，测试网可以暂缺，主网前补上）。
  Infura 的 Arbitrum Sepolia 节点**不支持 `eth_simulateV1`**（`-32601`，2026-10-20 实测）；forum-sponsor 会自动改用
  EntryPoint `eth_call` 模拟（`backend/bins/sponsor/src/sim.rs`），功能不受影响，只是每个节点冷却期内日志里提示一次。
  NodeInterface（L1 数据费估算）、跨 2000 块的 `eth_getLogs`、WebSocket 都可用。

## 一、注册服务与密钥

1. **Infura**（主节点）：infura.io 建一个 API key，在该 key 的网络设置里**启用 Arbitrum Sepolia**（没启用时请求返回错误，
   `preflight` 的「链 ID」一项失败）。后台支持的话设每日用量上限。
   地址：`https://arbitrum-sepolia.infura.io/v3/<key>`，WebSocket `wss://arbitrum-sepolia.infura.io/ws/v3/<key>`。
2. **备用节点（可选）**：Alchemy（`https://arb-sepolia.g.alchemy.com/v2/<key>`，在 app 里启用 Arbitrum Sepolia）或其他服务商。
   免费额度用完的账号不要填（`preflight` 会把它当作故障节点）。
3. **Arbiscan 源码验证**：Etherscan 账号的 API key（Etherscan API v2，一个 key 通用于各条链）。免费档覆盖哪些链会调整，
   验证报套餐 / 权限错误时看 Etherscan 后台说明（部署本身不受影响，之后可补验证）。
4. **（可选）备用 bundler**：Pimlico 的 API key，填进服务器 `forum-sponsor.env` 的 `FALLBACK_BUNDLER_URL`。

把这些写进**仓库以外**的一个文件（例如 `~/secrets/akari-arbitrum-sepolia.env`），权限 0600：

```sh
install -m 600 /dev/null ~/secrets/akari-arbitrum-sepolia.env
$EDITOR ~/secrets/akari-arbitrum-sepolia.env
```

```ini
ARBITRUM_SEPOLIA_RPC_URL=https://arbitrum-sepolia.infura.io/v3/<key>
ARBITRUM_SEPOLIA_WS_URL=wss://arbitrum-sepolia.infura.io/ws/v3/<key>
# ARBITRUM_SEPOLIA_RPC_BACKUP_URL=https://arb-sepolia.g.alchemy.com/v2/<key>
ARBISCAN_API_KEY=<key>
```

`testnet.sh` 只读这几行（不 source、不执行），经环境变量交给 forge / cast，URL 不出现在进程命令行里；所有输出经过遮盖，
密钥显示为 `<redacted>`。forge 为 `--resume` 在 `contracts/cache/Deploy.s.sol/421614/` 记下带密钥的 RPC 地址，
脚本把它设成只有本人可读、部署成功后删除。

## 二、部署账户

在自己的电脑上（不是服务器）建一个只用于测试网部署的密钥库：

```sh
cast wallet new ~/.foundry/keystores akari-testnet-deployer   # 交互输入口令；或 cast wallet import akari-testnet-deployer --interactive
cast wallet list                                              # 记下地址，填 DEPLOYER_ADDRESS
```

用 Ledger 时设 `DEPLOYER_LEDGER=1`（用第一个账户）。脚本从不接受明文私钥；无人值守时可用 `DEPLOYER_PASSWORD_FILE`（0600）。

**领测试网 ETH**：部署账户约需 0.13 ETH（部署 gas 约 0.001 ETH + Paymaster 押金 0.1 + 热钱包 0.02，`preflight` 按当时
gas 价算出准确数字），运营 Safe 另需 0.01 ETH 质押。来源：各 RPC 服务商与 Chainlink 等的 Arbitrum Sepolia 水龙头（多数要求
主网地址有少量余额，规则常变），或在 Sepolia 领 ETH 后经官方桥 `bridge.arbitrum.io` 转到 Arbitrum Sepolia（约 10 分钟）。
测试网 ETH 紧张时可以一起调低 Paymaster 押金 / 热钱包目标值与 forum-sponsor 告警线（见 `.env.example`）。

## 三、forum-sponsor 的热密钥

三把密钥在可信的电脑上生成，私钥经 SSH 直接送进服务器的 `systemd-creds encrypt`，不落盘、不进任何文件：

```sh
umask 077
for name in paymaster-signer relayer-hot-wallet auction-signer; do
  cast wallet new --json >/dev/shm/$name.json                 # tmpfs，0600
  jq -r '(.data // .) | (if type == "array" then .[0] else . end) | .address' /dev/shm/$name.json   # 记下地址
  jq -r '(.data // .) | (if type == "array" then .[0] else . end) | .private_key' /dev/shm/$name.json |
    ssh <测试服务器> "sudo systemd-creds encrypt --name=$name - /etc/akari/credstore/$name.cred"
  shred -u /dev/shm/$name.json
done
```

地址分别填 `PAYMASTER_SIGNER`、`RELAYER_ADDRESS`、`AUCTION_SIGNER`。`auction-signer` 是可选凭据（deploy/README.md）：
不开拍卖时可以不生成，`AUCTION_SIGNER` 留空。

## 四、建 Safe（app.safe.global）

每个签名人先准备好自己的钱包（硬件钱包、浏览器钱包或 Passkey 均可；同一个人不要用同一个 iCloud / Google 账号下的多个 Passkey）。
由任一签名人操作，连接钱包付建 Safe 的 gas：

1. 打开 https://app.safe.global ，右上角网络选 **Arbitrum Sepolia**，连接钱包。
2. **Create account** → 名称（如「Akari 测试网仲裁」）→ 网络确认是 Arbitrum Sepolia。
3. **Signers**：逐个填签名人地址；**Threshold** 按上表（仲裁 2/3、金库 2/3、运营 1/N）。
4. 确认并部署，等交易上链，记下 Safe 地址（`arb-sep:0x…` 冒号后的部分）。
5. 重复建另外两个 Safe。运营 Safe 建好后往里转 0.01 ETH（Paymaster 质押从它转出）。

填进 `deploy/testnet/.env`（`cp deploy/testnet/.env.example deploy/testnet/.env`）：

```ini
ARBITER_SAFE=0x…
ARBITER_SAFE_OWNERS=0x管理员A,0x管理员B,0x站长
ARBITER_SAFE_THRESHOLD=2
TREASURY_SAFE=0x…
TREASURY_SAFE_OWNERS=0x设备1,0x设备2,0x设备3
TREASURY_SAFE_THRESHOLD=2
OPS_SAFE=0x…
OPS_SAFE_OWNERS=0x站长
OPS_SAFE_THRESHOLD=1
```

`preflight` / `verify` 逐个核对：地址有代码且是 Safe、门槛与签名人集合（不分顺序与大小写）与这里一致、签名人里没有部署账户、
仲裁 Safe 不是金库 Safe。签名人以后有变动（换人、丢设备），先在 Safe 里改，再同步改这里。

## 五、部署

```sh
deploy/testnet/testnet.sh preflight     # 只读：RPC（链 ID、延迟、eth_simulateV1、NodeInterface、eth_getLogs）、AA 合约、Safe、签名者、余额
deploy/testnet/testnet.sh deploy        # 先自动跑 preflight；forge script 部署 5 个合约（有 ARBISCAN_API_KEY 时同时验证源码）
deploy/testnet/testnet.sh configure     # 部署账户补押金与热钱包；生成运营 Safe 的质押交易文件
#   → 在 Safe 里执行生成的 Transaction Builder 文件（见下）
deploy/testnet/testnet.sh configure     # 再跑一次：应提示「链上已与配置一致」
deploy/testnet/testnet.sh verify        # 链上断言，全部 ✓ 才算完成
deploy/testnet/testnet.sh emit-config   # 写前端 / 后端链配置
```

- `deploy` 只发生一次：已有部署记录就拒绝运行（重新部署见下文）。中途断掉（RPC 超时、余额不足）时用 `deploy --resume` 续发。
  源码验证失败（Arbiscan 偶尔超时）不影响部署记录，之后在 `contracts/` 下用同一份环境再跑
  `forge script script/Deploy.s.sol --rpc-url arbitrum_sepolia --resume --verify` 补验证。
- **执行 Safe 交易文件**：`configure` 把需要 Safe 签的交易写成 `deployments/arbitrum-sepolia/safe-tx/*.json`（与 forum-api
  给仲裁签名人的文件同一格式）。app.safe.global → 对应 Safe → **Apps → Transaction Builder** → 把 JSON 拖进去 →
  核对每笔的目标地址、金额与说明 → **Create batch → Send batch** → 其他签名人在 Queue 里确认 → 执行。首次部署只有一笔：
  运营 Safe 调 `FixedFeePaymaster.addStake(86400)`，附 0.01 ETH（质押只有公共 bundler 备用通道需要；私有代发器不需要）。
  以后改了 `PAYMASTER_SIGNER`（换代付签名者）或 `AUCTION_SIGNER`，`configure` 会生成 `setSigner` / `queueAuctionSigner`；
  后者排队 72 小时，到期后再跑一次 `configure` 由部署账户执行 `applyAuctionSigner`。
- `verify` 核对：五个合约的链上代码与本地构建一致（immutable 位置清零后逐字节比较）；EscrowConfig 的 owner = 金库 Safe、
  没有待接受的 owner 转移、参数等于 plan 第 9 节初始值（费率 1%、最低手续费 1、最低订单 5、**单笔上限 200、总额上限 5000 USDT**、
  gas 费 0.1 / 0.02、争议费 1 倍、仲裁超时 30 天、和解 72 小时）、没有排队中的参数、`DELAY` = 72 小时、未暂停、无免费活动、
  代币在白名单、两个市场已登记、拍卖签名者；两个市场的 arbiter = 仲裁 Safe、feeRecipient = 金库 Safe；Paymaster 的 EntryPoint、
  代币、treasury = 金库 Safe、owner = 运营 Safe、signer = `PAYMASTER_SIGNER`、`MAX_FEE` 1 USDT、`MAX_VALIDITY` 10 分钟、押金、
  质押与解押延迟；热钱包余额；Safe 与签名者、AA 基础合约（同 preflight）。
- `emit-config` 写两份文件（只有地址，不含任何密钥）：
  - `frontend/packages/contracts/src/chain.arbitrum-sepolia.json`：前端链配置，形状同 `chain.local.json`，另含
    `walletInitVersions`。**钱包初始化参数版本只增不改**（ADR 0008）：文件已存在时沿用其中的版本，Paymaster 或代币变了才追加新版本。
  - `deploy/testnet/deployments/arbitrum-sepolia/chain.toml`：后端 `CHAIN_CONFIG`。
- 完成后提交 `deploy/testnet/deployments/arbitrum-sepolia/` 与前端链配置（广播记录里只有交易与地址，没有 RPC 地址或密钥）。

## 六、测试服务器

`/etc/akari/` 下（deploy/README.md「环境变量文件」）：

1. 链配置：`sudo install -m 644 deploy/testnet/deployments/arbitrum-sepolia/chain.toml /etc/akari/chain.toml`（没有密钥）。
2. `forum-api.env`、`forum-worker.env`、`forum-indexer.env`、`forum-sponsor.env`（都是 0600）：
   ```ini
   CHAIN_CONFIG=/etc/akari/chain.toml
   CHAIN_RPC_PRIMARY_HTTP=https://arbitrum-sepolia.infura.io/v3/<key>
   CHAIN_RPC_PRIMARY_WS=wss://arbitrum-sepolia.infura.io/ws/v3/<key>     # forum-indexer 用于即时事件；其他程序可不设
   # CHAIN_RPC_BACKUP_HTTP=… / CHAIN_RPC_BACKUP_WS=…
   ```
   forum-api 与 forum-worker 另设 `SPONSOR_URL=http://127.0.0.1:8090` 与同一个 `SPONSOR_API_TOKEN`（≥ 32 字符，
   `openssl rand -hex 32`）；forum-api 设 `ARBITRATION_SAFE=<仲裁 Safe>`。forum-sponsor 设 `SPONSOR_API_TOKEN`，可选
   `FALLBACK_BUNDLER_URL`（Pimlico，带 key）、`SPONSOR_ALERT_DEPOSIT_WEI` / `SPONSOR_ALERT_HOT_WALLET_WEI`（调低了告警线时）。
3. 凭据：第三步已生成 `paymaster-signer`、`relayer-hot-wallet`（与可选的 `auction-signer`）的 `.cred`；重跑 Ansible play
   让单元加上可选凭据，`systemctl restart forum-sponsor forum-api@1 forum-api@2 forum-worker forum-indexer`。
4. 检查：
   - `journalctl -u forum-sponsor`：启动时核对拍卖签名者与链上一致；`eth_simulateV1 unavailable, switching this endpoint
     to EntryPoint simulation` 是 Infura 的正常提示；没有 `balance alert raised`。
   - `curl -s 127.0.0.1:8090/healthz`：`relayer: true`。forum-sponsor 启动时的链上核对遇到 RPC 超时会直接退出（演练里
     fork 来源偶尔慢时出现过一次），由 systemd 重启即可。
   - forum-indexer 日志出现 `reconciled`，`to` 跟上链头。
   - 在任何一台装了前端依赖的电脑上做只读冒烟（钱包 SDK 推算的地址 = 链上 Kernel 工厂，initCode 授权本版本 Paymaster）：
     ```sh
     cd frontend/apps/wallet
     CHAIN_SMOKE_JSON=../../packages/contracts/src/chain.arbitrum-sepolia.json \
       CHAIN_SMOKE_RPC=https://sepolia-rollup.arbitrum.io/rpc pnpm exec vitest run lib/wallet/chain-smoke.test.ts
     ```

**RPC 用量**（2026-10-20 演练时用 `rpc-meter.py` 实测；credit 按 Infura 公布的每方法计费表估算：`eth_getLogs` 255、
`eth_chainId` 5、其余常用读方法 80，以官网为准）：

| 场景 | 请求 | 约合 Infura credits |
| --- | --- | --- |
| `preflight` 一次（未填 Safe 时；填了再多十来次 `eth_call`） | 30 | 2 千 |
| 四个后端程序空转 313 秒（indexer 每 5 秒补拉：`eth_getBlockByNumber` 147、`eth_getLogs` 54，其余个位数） | 238 | 2.9 万 |
| 折合一整天（`reconcile_interval_ms = 5000`） | 约 5.5 万 | **约 700 万** |
| forum-sponsor 代付一笔操作（模拟、gas 估算与价格；不含提交） | 约 12 | 1 千 |

Infura 免费档每天 300 万 credits（以官网为准）：按默认 5 秒补拉，单是 indexer 就超出。测试网可以在 `deploy/testnet/.env`
设 `INDEXER_RECONCILE_MS=20000` 后重跑 `emit-config`（约 175 万 / 天；WebSocket 订阅照常即时显示，补拉只负责校对），
或者升级套餐、加第二家节点分摊。主网的用量与套餐另行评估（plan 第 2 节「indexer 省调用」）。WebSocket 订阅推送的计费没有计入。

## 七、回滚与重新部署

合约不可升级，「回滚」就是停用旧合约、部署新合约：

- **暂停新订单**：金库 Safe 调 `EscrowConfig.setPaused(true)`（立即生效，不排队；旧订单照常走完）。
- **换代付签名者**（热密钥泄露）：生成新密钥、更新凭据与 `PAYMASTER_SIGNER` → `configure` 生成 `setSigner` → 运营 Safe 执行 →
  重启 forum-sponsor。热钱包泄露：换新地址、更新 `RELAYER_ADDRESS` 与凭据，旧热钱包剩余 ETH 自行转走。
- **取回 Paymaster 押金**：运营 Safe 调 `withdrawTo(地址, 金额)`；质押要先 `unlockStake`，等解押延迟（1 天）后 `withdrawStake`。
- **重新部署**：
  1. `git mv deploy/testnet/deployments/arbitrum-sepolia deploy/testnet/deployments/arbitrum-sepolia-<日期>`（保留旧记录）。
  2. **不要删前端链配置**：`emit-config` 要读其中已发出的钱包初始化参数版本；换了 Paymaster 或代币时它追加新版本，
     老钱包继续用旧版本（ADR 0008）。旧 Paymaster 的押金在没有未部署的老钱包依赖它之前不要取走。
  3. 重新 `preflight → deploy → configure → verify → emit-config`，更新服务器上的 `chain.toml` 并重启。
  4. 目前 indexer 只认一组合约地址：旧合约上的订单不再进论坛数据库。测试网可以清空测试数据库后从新的 `start_block` 补拉；
     主网的「新旧合约同时识别」（plan 第 13 节合约迁移）还没实现，上线前要补并在测试网演练。
- **只改链下配置**（RPC 换服务商、加备用节点）：改 `/etc/akari/*.env` 后重启，不碰合约。

## 八、上线清单对照（plan 第 16 节）

| plan 第 16 节条目 | 测试网这一步提供的 | 还要另外做 |
| --- | --- | --- |
| 主网地址源码已验证；总额与单笔上限生效 | `deploy --verify`；`verify` 断言上限 200 / 5000 | 主网另跑一次（`Deploy.s.sol` 在 42161 上强制 Safe 检查） |
| `resolveTimeout`、`withdraw(to)`、`cancelOffer` 已在测试网演练 | 合约与 Safe 都在测试网，仲裁 Safe 签名人已就位 | 按清单逐项演练并记录 |
| 仲裁、金库、运营、薪资 Safe 已建；owner 已移交金库 Safe；签名人演练批量 resolve | 三个 Safe 照主网分工建；EscrowConfig owner 构造时即为金库 Safe；Transaction Builder 流程在 `configure` 里走过一遍 | 薪资 Safe；批量 resolve 演练 |
| 私有代发器、Paymaster 押金与代付签名服务上线，备用 bundler 切换实测、余额告警；固定 gas 费按测试网实测成本确定 | `configure` 押金 / 质押 / 热钱包；`preflight` 校验目标值高于告警线；部署 gas 实测 | 备用 bundler 切换实测；按测试网实际 gas 校准 `baseGas` / `claimGas` |
| 链配置（USDT 地址、精度、最低费用）核对无误 | `emit-config` 由部署记录生成，`verify` 核对代币与参数 | 主网换成 USD₮0 地址 |
| 紧急取回实测 | 前端链配置带公共 RPC / bundler，紧急取回页可连测试网 | 停机实测一次 |
| 拍卖已在测试网演练 | `AUCTION_SIGNER` 写入 EscrowConfig，forum-sponsor 启动时核对 | 按清单演练 |
| 前端产物里找不到第三方 Bundler / RPC 密钥 | 前端链配置只含公共地址；密钥只在 `RPC_SECRETS_FILE` 与服务器 env 文件 | 构建后扫描产物 |
| 卖家保证金、备用仲裁通道、合约迁移已在测试网演练 | 部署与重新部署步骤（第七节） | 合约迁移需要 indexer 支持新旧地址 |

## 演练（本机 fork）

```sh
RPC_SECRETS_FILE=~/secrets/akari-arbitrum-sepolia.env deploy/testnet/rehearse.sh up   # 不设时 fork 公共 RPC
deploy/testnet/rehearse.sh run        # preflight → deploy → configure → 冒充 Safe 执行 → configure → verify → emit-config
deploy/testnet/rehearse.sh services   # 独立 PostgreSQL 容器（--memory 2g）+ 四个后端程序
deploy/testnet/rehearse.sh smoke      # forum-api 初始化参数 = 前端链配置；钱包地址核对；forum-sponsor 模拟并签一笔付款
deploy/testnet/rehearse.sh down       # 停进程、删容器，打印上游 RPC 各方法请求数
```

anvil 在 127.0.0.1:8345 fork（链 ID 保持 421614，与真实部署走同一条代码路径；fork 来源经 127.0.0.1:8346 的计数转发），
Safe 用链上现成的 Safe v1.4.1 工厂真实创建，部署账户是一次性密钥库，热密钥一次性生成，全部只在 `.run/rehearsal/`。
差异：anvil 没有 Arbitrum 的 NodeInterface（换成把 L1 数据费估成 0 的替身）、不算 L1 数据费；`eth_simulateV1` 在 anvil 上可用，
所以 forum-sponsor 走的是首选模拟方法（真实 Infura 上走备用方法）；Safe 交易用 anvil 冒充 Safe 地址执行，不走签名人流程。
端口可用 `REHEARSAL_*_PORT` 改。
