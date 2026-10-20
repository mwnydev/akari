# 部署

单台裸机 + systemd（plan 第 13 节）。服务器上不编译，只安装 CI 构建并带 GitHub 构建证明的产物。

| 路径 | 内容 |
| --- | --- |
| `dev/compose.yaml` | 本机开发依赖（PostgreSQL 18 + PGroonga、Valkey、Mailpit） |
| `ansible/` | 服务器初始化：base、cloudflared、firewall、postgresql、valkey、caddy、app 七个角色 |
| `install-release.sh` | 安装 / 回滚版本；部署账号经 sudoers 只能执行它 |
| `check-origin-exposure.sh` | 从外部主机检查源站 IP 是否暴露；每次部署、改 DNS 或换 IP 后运行 |

## 初始化一台服务器

```sh
cd deploy/ansible
ansible-galaxy collection install -r requirements.yml -p collections
ansible-playbook -i inventory/staging.yml site.yml      # 或 inventory/production.yml
```

前提：Debian 13、`deploy` 账号已有 SSH 公钥和 sudo（首次运行后 sudo 被收窄为只能执行 install-release，后续改服务器配置用单独的运维账号）。

**源站 IP 保护**（[docs/runbooks/origin-protection.md](../docs/runbooks/origin-protection.md)）：公网不开任何端口，网页与 SSH 都经 Cloudflare Tunnel，
SSH 另需 Cloudflare Access 登录；源站 IP 不进仓库。上面的命令是日常用法（经隧道连接，本机需装 `cloudflared`）；
新服务器第一次运行、隧道故障应急、DNS 清单见该手册，换 IP 见 [origin-ip-rotation.md](../docs/runbooks/origin-ip-rotation.md)。

## 角色要点

- **base**：自动安全更新；SSH 只允许密钥、非默认端口 22022；大页、`vm.overcommit_memory=1`。
- **cloudflared**：锁版本的 Cloudflare Tunnel 连接器（sha256 校验）两个实例，systemd 沙箱；隧道凭据用 `LoadCredentialEncrypted`；验证经隧道能登录后 sshd 只听本机。
- **firewall**：nftables。入站：tunnel 模式不放行任何端口（direct 备用模式只放行 Cloudflare 网段的 443）；公网 SSH 只在 `firewall_ssh_bootstrap_allow` 临时放行。出站只放行 443、DNS、NTP、隧道等固定端口。
- **postgresql**：PGDG 18 + PGroonga + pgBackRest；只监听 Unix socket；参数按 `group_vars/<环境>.yml`；只设 `MemoryHigh`。每个程序一个数据库账号，授权见下方「数据库授权矩阵」。
- **valkey**：官方 9.1.2 二进制（sha256 校验），只监听 Unix socket，AOF，`volatile-lru`。
- **caddy**：tunnel 模式只听本机、未知 Host 直接断开；direct 模式用 Cloudflare Origin CA 证书 + Authenticated Origin Pulls（`require_and_verify`）；真实 IP 只信任上一跳的 `CF-Connecting-IP`；api 两实例 `least_conn` + 健康检查；Next.js 限并发。
- **app**：每个程序独立系统用户和 systemd 单元（`ProtectSystem=strict`、`NoNewPrivileges`、`MemoryMax` 等）；forum-sponsor 的私钥用 `LoadCredentialEncrypted`（`systemd-creds encrypt` 生成，放 `/etc/akari/credstore/`）。

## 环境变量文件（/etc/akari/*.env，不进仓库）

| 文件 | 关键变量 |
| --- | --- |
| `forum-api.env` | `TURNSTILE_SECRET`（Cloudflare Turnstile 密钥，staging / production 必填，缺了拒绝启动；文件权限 0600，与 `COOKIE_SECRET` 一样只放这里）、`DATABASE_URL`、`VALKEY_URL`（Unix socket）、`COOKIE_SECRET`（64 字节十六进制）、`AUTH_SECRET`、`BRAND_NAME`、`ARBITRATION_SAFE`（仲裁 Safe 地址，写进给签名人导入的 Safe 交易文件）；域名见下方「域名」，生产不在这里配置（设置了反而拒绝启动），测试服务器在这里设 `ALLOWED_ORIGINS`、`INTERNAL_HOSTS`、`WALLET_RP_ID`（钱包中心域名）、`WALLET_ORIGINS`（钱包子域名列表，绝不含论坛主站）、`ADMIN_RP_ID` / `ADMIN_ORIGIN`（站长后台 Passkey 的 RP ID 与后台页面来源，ADR 0013）、`RULING_RP_ID` / `RULING_ORIGIN`（论坛裁决 Passkey：RP ID 是裁决页面的主机，裁决页面须在 `ALLOWED_ORIGINS` 里，RP ID 绝不能等于 `WALLET_RP_ID`）；`CHAIN_CONFIG`（链配置文件路径，与 forum-indexer / forum-sponsor 同一份，格式见 `backend/crates/chain/chain.example.toml`；链 ID、合约、USDT 与钱包初始化参数版本都取自它，RPC 用 `CHAIN_RPC_PRIMARY_HTTP` / `CHAIN_RPC_BACKUP_HTTP` 提供，未设置时才用 `CHAIN_ID`、`ESCROW_ADDRESS`、`USDT_ADDRESS` 且钱包地址不能登记）；`SPONSOR_URL`（本机 forum-sponsor，如 `http://127.0.0.1:8090`）与 `SPONSOR_API_TOKEN`（与 forum-sponsor 相同的共享令牌，≥ 32 字符），两项都设置才开放 `/api/v1/aa/rpc` |
| `forum-worker.env` | `DATABASE_URL`、`VALKEY_URL`、`MAIL_FROM`、邮件通道：生产只用 HTTP API，`MAIL_PRIMARY=resend`（`RESEND_API_KEY`）、`MAIL_BACKUP=postmark`（`POSTMARK_SERVER_TOKEN`）；SMTP（`SMTP_URL`）只能连本机（开发 Mailpit），因为 SMTP 会在邮件头里留下源站 IP；管理员群告警 `TELEGRAM_BOT_TOKEN`（BotFather 发的机器人令牌，密钥，日志里不出现）与 `TELEGRAM_ADMIN_CHAT_ID`（管理员群的 chat id，如 `-100…`，机器人须在群里），两项都设置才启用，都不设置则告警关闭（启动时警告），只设一项拒绝启动；推送失败按发件箱指数退避重试；可选 `ARBITRATION_RULING_DEADLINE_HOURS`（仲裁员接案后多久不裁决就改派，默认 48）；拍卖结算（结束、违约禁拍、次高者递补）要 `CHAIN_CONFIG` + `CHAIN_RPC_*`（读出价资格余额与链上 `auctionSettled`）与 `SPONSOR_URL` + `SPONSOR_API_TOKEN`（向 forum-sponsor 请求成交证明，同 forum-api 的令牌），三项都设置才启用、都不设置则拍卖不结算（启动时警告）、只设一部分拒绝启动；可选 `AUCTION_DEFAULT_GRACE_SECS`（付款截止后再等多久才判违约，默认 300，60–3600） |
| `forum-indexer.env` | `DATABASE_URL`、`CHAIN_CONFIG`、`CHAIN_RPC_PRIMARY_HTTP` / `_WS`、`CHAIN_RPC_BACKUP_HTTP` / `_WS` |
| `forum-sponsor.env` | `DATABASE_URL`、`CHAIN_CONFIG`（必须含 `[aa]`：EntryPoint、Paymaster、允许的 Kernel 工厂）、`CHAIN_RPC_*`、`SPONSOR_LISTEN`（只听本机，默认 `127.0.0.1:8090`）、`SPONSOR_API_TOKEN`；可选 `SPONSOR_FEE`、`SPONSOR_BATCH_WINDOW_MS`、`SPONSOR_MAX_OP_COST_WEI`、`SPONSOR_ALERT_DEPOSIT_WEI`、`SPONSOR_ALERT_HOT_WALLET_WEI`、`FALLBACK_BUNDLER_URL`（备用 bundler，带服务商密钥）。私钥不放这里：`paymaster-signer`、`relayer-hot-wallet` 走 systemd 凭据；拍卖成交证明的 `auction-signer`（`EscrowConfig.auctionSigner` 对应的 EOA 私钥，只能签成交证明）也走 systemd 凭据，可选：没有时 forum-sponsor 不签成交证明（启动时警告），有时启动核对它就是链上的 `auctionSigner`。开放拍卖前：`systemd-creds encrypt --name=auction-signer - /etc/akari/credstore/auction-signer.cred`，再把 `auction-signer` 加进 `group_vars/all.yml` 里 forum-sponsor 的 `credentials`（列表里的凭据文件不存在时单元起不来，所以默认不列） |
| `web.env` | `COOKIE_SIGNING_KEY` = `COOKIE_SECRET` 的**前 32 字节**（只给签名那一半，最小权限）、`API_ORIGIN` |

`/etc/akari/deployment.env` 由 Ansible 写入（`AKARI_ENV`，取自 `group_vars/<环境>.yml` 的 `akari_env`），排在各程序的 `.env` 之后，手写的 `.env` 覆盖不了它。`KV_PREFIX` 只用于多个部署共用一个 Valkey（本机开发），生产不设。

## 数据库授权矩阵

`roles/postgresql/templates/grants.sql.j2` 按程序列出 `forum_api`、`forum_worker`、`forum_indexer`、`forum_sponsor`
四个账号能碰的每张表与权限（能收窄到列的收窄到列，如 forum-api 对 `outbox` 只能插入、只能读 `id / dedupe_key / created_at`，
读不到验证码等 payload；forum-worker 对 `topics` 只能改 `view_count`）。脚本一个事务内先收回四个账号的全部权限再按矩阵授予，
可重复执行，跑完即与矩阵一致（多出来的旧授权会被收回）。

矩阵由 `backend/crates/db/tests/grants.rs` 在 CI 里核对：从各程序源码推出它会调用的 forum-db 函数及其 SQL，
在迁移完成的临时库上套用本脚本，以各账号对每条 SQL 做 `EXPLAIN`（PostgreSQL 照常检查表与列权限）；
新查询用到没授权的表或列、或矩阵里有某程序用不到的表与权限，测试都会失败。改矩阵或加迁移后把
`group_vars/all.yml` 的 `pg_grants_schema_version` 改成最新迁移的版本号；Ansible 不跑迁移，新机器首次迁移后再跑一次 play。

## Turnstile 与内容安全策略

- 站点密钥（公开）在论坛构建时内联：`NEXT_PUBLIC_TURNSTILE_SITE_KEY`（前端 `.env.production`）；密钥 `TURNSTILE_SECRET` 在 `forum-api.env`。
  Cloudflare 后台把站点密钥限定在 `akari.cc`。
- forum-api 经 `https://challenges.cloudflare.com/turnstile/v0/siteverify` 校验（出网 443 已放行，客户端登记在出网测试里，5 秒超时），
  并核对令牌的 `action`（登录表单 `login`、发帖 `post`），Cloudflare 不可用时拒绝而不是放行。
- 论坛目前不发 CSP。以后加 CSP 时须放行 `script-src https://challenges.cloudflare.com` 与 `frame-src https://challenges.cloudflare.com`
  （脚本只在登录页与新用户的发帖表单上按需插入）。

## 域名（ADR 0012）

| 用途 | 地址 | 部署 |
| --- | --- | --- |
| 论坛主站（裁决 Passkey RP ID `akari.cc`） | `https://akari.cc` | 本服务器（Caddy `site_domain`，经 Cloudflare） |
| 钱包（Passkey RP ID `wallet.akari.cc`） | `https://wallet.akari.cc` | Cloudflare Pages |
| 后台（Passkey RP ID `administrator.akari.cc`） | `https://administrator.akari.cc` | Cloudflare Pages |

- 只经代码审查与发布流程修改，不进环境变量、不进后台在线配置：forum-api 在 `AKARI_ENV=production` 下用代码里的常量（`backend/bins/api/src/config.rs` 的 `domains`）；前端构建时取各应用的 `.env.production`。
- forum-api 启动检查：来源必须是 https（只有 `AKARI_ENV=development` 允许本机 http 与 `COOKIE_SECURE=false`）、钱包 / 后台来源不能同时是论坛来源（后台也不能是钱包来源）、钱包 / 后台的 Passkey RP ID 必须覆盖自己的页面、两者不同，且不能是论坛主站或其父域名（`akari.cc` 永远不行）；裁决 Passkey 的 RP ID 是裁决页面（论坛来源）的主机、绝不等于钱包 RP ID；`AKARI_ENV` 只认 `production` / `staging` / `development`，拼错拒绝启动。
- 会话 Cookie 保持 host-only，永远不设 `Domain`（`backend/bins/api/tests/auth_flow.rs` 有测试锁定）。

## 尚未完成

- pgBackRest 归档到 R2（等 R2 凭据）、Prometheus / Grafana / Loki、AIDE 基线初始化。
- 隧道 UUID、Access AUD、测试服务器域名待填（`group_vars/<环境>.yml`）；direct 模式的证书是占位路径。
