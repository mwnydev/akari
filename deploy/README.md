# 部署

单台裸机 + systemd（plan 第 13 节）。服务器上不编译，只安装 CI 构建并带 GitHub 构建证明的产物。

| 路径 | 内容 |
| --- | --- |
| `dev/compose.yaml` | 本机开发依赖（PostgreSQL 18 + PGroonga、Valkey、Mailpit） |
| `ansible/` | 服务器初始化：base、firewall、postgresql、valkey、caddy、app 六个角色 |
| `install-release.sh` | 安装 / 回滚版本；部署账号经 sudoers 只能执行它 |

## 初始化一台服务器

```sh
cd deploy/ansible
ansible-galaxy collection install -r requirements.yml -p collections
ansible-playbook -i inventory/staging.yml site.yml      # 或 inventory/production.yml
```

前提：Debian 13、`deploy` 账号已有 SSH 公钥和 sudo（首次运行后 sudo 被收窄为只能执行 install-release，后续改服务器配置用单独的运维账号）。

## 角色要点

- **base**：自动安全更新；SSH 只允许密钥、非默认端口 22022；大页、`vm.overcommit_memory=1`。
- **firewall**：nftables，80/443 只放行 Cloudflare 网段（运行时从 cloudflare.com 拉取）。
- **postgresql**：PGDG 18 + PGroonga + pgBackRest；只监听 Unix socket；参数按 `group_vars/<环境>.yml`；只设 `MemoryHigh`。
- **valkey**：官方 9.1.2 二进制（sha256 校验），只监听 Unix socket，AOF，`volatile-lru`。
- **caddy**：Cloudflare Origin CA 证书 + Authenticated Origin Pulls（`require_and_verify`）；真实 IP 只信任 Cloudflare 网段的 `CF-Connecting-IP`；api 两实例 `least_conn` + 健康检查；Next.js 限并发。
- **app**：每个程序独立系统用户和 systemd 单元（`ProtectSystem=strict`、`NoNewPrivileges`、`MemoryMax` 等）；forum-sponsor 的私钥用 `LoadCredentialEncrypted`（`systemd-creds encrypt` 生成，放 `/etc/akari/credstore/`）。

## 环境变量文件（/etc/akari/*.env，不进仓库）

| 文件 | 关键变量 |
| --- | --- |
| `forum-api.env` | `DATABASE_URL`、`VALKEY_URL`（Unix socket）、`COOKIE_SECRET`（64 字节十六进制）、`AUTH_SECRET`、`ALLOWED_ORIGINS`、`INTERNAL_HOSTS`、`WALLET_RP_ID`（钱包中心域名）、`WALLET_ORIGINS`（钱包子域名列表，绝不含论坛主站）、`BRAND_NAME`、`CHAIN_ID`、`ESCROW_ADDRESS`、`USDT_ADDRESS` |
| `forum-worker.env` | `DATABASE_URL`、`VALKEY_URL`、邮件通道（Resend / SMTP） |
| `web.env` | `COOKIE_SIGNING_KEY` = `COOKIE_SECRET` 的**前 32 字节**（只给签名那一半，最小权限）、`API_ORIGIN` |

## 尚未完成

- pgBackRest 归档到 R2（等 R2 凭据）、Prometheus / Grafana / Loki、AIDE 基线初始化。
- 证书、域名、主机地址是占位值，域名确定后填写。
