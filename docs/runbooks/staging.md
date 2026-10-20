# sslip 测试服务器（staging）

一台一次性的测试服务器，接 Arbitrum Sepolia 上已部署的合约（[testnet-deploy.md](testnet-deploy.md)「当前部署」），给站长在浏览器里试用。
还没有 Cloudflare 账号，所以和生产的做法有几处不同，全部只在 `akari_env: staging` 下可用（`site.yml` 开头断言，生产选不到）：

| | 生产（[origin-protection.md](origin-protection.md)） | sslip 测试服务器 |
| --- | --- | --- |
| 域名 | `akari.cc` 等（代码常量，ADR 0012） | `forum.` / `wallet.` / `admin.` / `media.` / `mail.` + `<ip-dashed>.sslip.io`（sslip.io 把名字解析成里面写的 IP），forum-api 从 `forum-api.env` 读 |
| 入口 | Cloudflare Tunnel，公网不开端口（`origin_ingress: tunnel`） | `origin_ingress: public`：22 / 80 / 443 对公网开放，Caddy 直接向 Let's Encrypt 申请证书 |
| SSH | 22022，经 Cloudflare Access，`deploy` 账号 | 22，sshd 保持 Debian 默认（站长决定：保留 root 密码登录，`ssh_hardening: false`，只能在 staging 关，`site.yml` 断言）；Ansible 以 root 用密钥连接，另建 `deploy` 账号（sudo 只能执行 install-release） |
| 钱包 / 后台 | Cloudflare Pages | Caddy 直接提供版本里的静态导出（`/opt/akari/current/wallet`、`admin`） |
| 邮件 | Resend + Postmark（HTTP API） | 本机 Mailpit（SMTP 127.0.0.1:1025），网页在 `mail.<ip-dashed>.sslip.io`（basic auth） |
| 图片 | R2 + CDN | 本机 SeaweedFS（S3 接口 127.0.0.1:8333），经 Caddy 以 `media.<ip-dashed>.sslip.io` 对外 |
| Turnstile | 正式密钥 | Cloudflare 文档公开的测试密钥（站点 `1x00000000000000000000AA`，总是通过）；forum-api 在 production 拒绝测试密钥 |
| 版本 | CI 构建 + GitHub 构建证明 | 本机构建（`deploy/staging/release.sh`），`install-release install --unattested`：只有服务器 `AKARI_ENV=staging` 且 Ansible 放了 `/etc/akari/allow-unattested-releases` 才接受 |

源站 IP 在这台服务器上是公开的（sslip 名字里就有），所以它**绝不能**和生产共用 IP、密钥或数据库。生产服务器的 IP 照旧保密。

## 文件与密钥在哪

控制机（运维自己的电脑，全部 0600，不进仓库）：

| 文件 | 内容 |
| --- | --- |
| `~/secrets/staging-host.txt` | 服务器 IP |
| `~/.ssh/akari_staging`、`~/.ssh/akari_staging_known_hosts` | root / deploy 的 SSH 私钥、固定的主机密钥 |
| `~/secrets/staging-root.pw` | root 密码（服务商控制台与 SSH 密码登录都可用） |
| `~/secrets/staging-mailpit.pw` | Mailpit 网页的密码（用户名 `owner`；改了要带 `-e mailpit_password_rotate=true` 重跑 play，否则只在第一次写入散列） |
| `~/secrets/infura-rpc.txt` | 第 2 行：Infura Arbitrum Sepolia HTTPS 地址（带密钥） |
| `~/secrets/keystore/akari-testnet-{paymaster-signer,relayer,auction-signer}` + `~/secrets/testnet-services.pw` | forum-sponsor 三把热密钥的密钥库与口令 |
| `deploy/ansible/inventory/staging-sslip.local.yml` | 带 IP 的 Ansible 清单（`.gitignore`，按下面生成） |

服务器：

| 路径 | 内容 |
| --- | --- |
| `/etc/akari/forum-{api,worker,indexer,sponsor}.env`、`web.env`（root 0600） | `akari-staging-env init` 写入；`COOKIE_SECRET`、`AUTH_SECRET`、`SPONSOR_API_TOKEN`、S3 访问密钥在服务器上生成，重跑保留 |
| `/etc/akari/seaweedfs-s3.json`（seaweedfs 0600） | S3 身份（匿名只读 + 一个读写身份） |
| `/etc/akari/credstore/{paymaster-signer,relayer-hot-wallet,auction-signer}.cred` | `systemd-creds encrypt` 加密的私钥（绑定本机密钥） |
| `/etc/caddy/mailpit-auth.caddy`（root:caddy 0640） | Mailpit basic auth 的 bcrypt 散列 |
| `/etc/akari/chain.toml` | `deploy/testnet/deployments/arbitrum-sepolia/chain.toml`（Ansible 复制） |

## 从零搭建

```sh
cd deploy/ansible
ansible-galaxy collection install -r requirements.yml -p collections
ip=$(head -1 ~/secrets/staging-host.txt)
umask 077
cat > inventory/staging-sslip.local.yml <<EOF
staging:
  children:
    staging_sslip:
      hosts:
        staging-sslip-1:
          ansible_host: $ip
          staging_sslip_base: ${ip//./-}.sslip.io
EOF
# Mailpit 网页口令（第一次）；Turnstile 测试密钥是公开的，放进一个临时文件交给 play
[ -f ~/secrets/staging-mailpit.pw ] || openssl rand -base64 24 | tr -d '/+=\n' > ~/secrets/staging-mailpit.pw
printf '1x0000000000000000000000000000000AA\n' > ../../.run/turnstile-test.secret
```

1. **初始化服务器**（全新 Debian 13，root 已有公钥）。第一次加载防火墙前先另开一个 SSH 连接留着，play 结束后再新开一个确认能登录：
   ```sh
   ansible-playbook -i inventory/staging-sslip.local.yml site.yml \
     -e turnstile_secret_file=../../.run/turnstile-test.secret -e mailpit_password_file=$HOME/secrets/staging-mailpit.pw
   ```
   第一次运行 SeaweedFS 还不会启动（身份文件要下一步生成），数据库授权也会跳过（还没迁移）。
2. **env 文件与服务器端密钥**（RPC 地址只经 stdin）：
   ```sh
   b=${ip//./-}.sslip.io
   sed -n 2p ~/secrets/infura-rpc.txt | ssh -i ~/.ssh/akari_staging -o UserKnownHostsFile=~/.ssh/akari_staging_known_hosts \
     root@$ip akari-staging-env init forum.$b wallet.$b admin.$b media.$b
   ```
3. **forum-sponsor 的热密钥**（私钥从密钥库解出后直接经 SSH 进 `systemd-creds`，不落盘、不出现在命令行参数里）：
   ```sh
   for pair in paymaster-signer:paymaster-signer relayer:relayer-hot-wallet auction-signer:auction-signer; do
     cast wallet private-key --keystore ~/secrets/keystore/akari-testnet-${pair%%:*} --password-file ~/secrets/testnet-services.pw |
       ssh -i ~/.ssh/akari_staging -o UserKnownHostsFile=~/.ssh/akari_staging_known_hosts root@$ip \
         "systemd-creds encrypt --name=${pair#*:} - /etc/akari/credstore/${pair#*:}.cred"
   done
   ```
4. **迁移**：`deploy/staging/release.sh migrate $ip`（本机 sqlx-cli 传上去，以 postgres 连接、`SET ROLE akari_owner`）。
5. **再跑一次第 1 步的 play**：套用数据库授权、启动 SeaweedFS、单元加上 `auction-signer` 凭据。
6. **构建并安装版本**：`deploy/staging/release.sh build ${ip//./-}.sslip.io && deploy/staging/release.sh install $ip`。
7. **种子数据**（可重复执行）：`deploy/staging/release.sh seed $ip`：板块（`backend/crates/db/seeds/dev.sql`）、打开拍卖；
   `~/secrets/akari-test-owner.email` 里的邮箱已注册时授予站长（role 1，邮箱只经 stdin 进 psql）。站长还没登录过就先让 Ta 登录一次再重跑。

play 中途失败时，已通知的 handler（重载 PostgreSQL / Caddy 等）不会执行，修好后重跑也不一定再触发：失败过一次就在服务器上
`systemctl reload postgresql@18-main caddy` 一次（2026-10-21 实测：pg_ident 映射没生效，服务全部 peer 认证失败）。

## 日常重新部署

改了代码：第 6 步（构建并安装）；加了迁移：先第 4 步再第 6 步，然后跑一次 play（`pg_grants_schema_version` 也要更新）；
改了 Ansible：第 1 步。回滚：`ssh deploy@$ip sudo /opt/akari/bin/install-release rollback`。

健康检查（服务器上）：

```sh
for i in 1 2; do curl -s 127.0.0.1:808$i/api/v1/healthz; echo; done
curl -s 127.0.0.1:8090/healthz          # forum-sponsor：relayer: true
journalctl -u forum-indexer -n 20       # reconciled，to 跟上链头
systemctl --failed
```

## 收邮件（登录验证码）

打开 `https://mail.<ip-dashed>.sslip.io`，用户名 `owner`，密码在 `~/secrets/staging-mailpit.pw`。测试服务器发出的所有邮件
（登录验证码、新设备提醒、通知）都只进这里，不会真的寄出。

## 已知差异与限制

- 钱包 / 后台的 Passkey 绑定在 sslip 名字上（RP ID `wallet.<ip-dashed>.sslip.io`、`admin.<ip-dashed>.sslip.io`），换 IP 或换域名后要重新注册。
- Let's Encrypt 每个注册域名每周有签发上限（sslip.io 在 Public Suffix List 上，按 `<ip-dashed>.sslip.io` 计），不要反复删证书重来。
- 公网 SSH 开着，并且接受 root 密码登录（站长决定）。服务器是一次性的：用完在服务商后台删除，或换成下面的正式测试域名。
- 没有备份（pgBackRest 未配置），没有 Telegram 告警，没有备用 bundler；RPC 主节点 Infura（HTTP + WebSocket），备用节点是 Arbitrum 官方公共 RPC（只有 HTTP）。

## 以后换成 staging.akari.cc（有 Cloudflare 账号之后）

1. 按 [origin-protection.md](origin-protection.md)「首次初始化」建隧道、Access 应用，填 `group_vars/staging.yml` 的
   `cloudflared_*` 与 `site_domain`（如 `staging.akari.cc`）。**新开一台服务器、新 IP**：这台的 IP 已经公开过。
2. 清单里不再用 `staging_sslip` 组（主机直接放在 `staging` 组下），`origin_ingress` 回到默认的 `tunnel`，`ansible_user` 回到 `deploy`。
3. 钱包、后台放到 Cloudflare Pages（如 `wallet-staging.akari.cc`、`administrator-staging.akari.cc`），图片用 R2，邮件用 Resend / Postmark；
   `forum-api.env` 里的 `ALLOWED_ORIGINS`、`WALLET_*`、`ADMIN_*`、`RULING_*` 改成新域名（规则见 deploy/README.md「域名」）。
4. 版本改由 CI 构建并带构建证明，`release_allow_unattested` 关闭。
