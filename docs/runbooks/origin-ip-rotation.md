# 更换源站 IP

源站 IP 泄露（暴露检查失败、源站直接被打、在邮件头或扫描结果里看到）时，换一台新 VPS，把服务迁过去，销毁旧机器。
前提与背景见 [origin-protection.md](origin-protection.md)。

**先找到泄露原因并修复**，否则新 IP 会以同样方式再泄露。服务器若可能已被入侵，不是本手册的范围：按 plan 第 11 节「应急预案」第 2 条
（轮换所有密钥、从干净备份重建），本手册的步骤可复用。

## 预计用时

| 步骤 | 用时 | 网站 |
| --- | --- | --- |
| 1. 开新 VPS、新隧道 | 15–20 分钟 | 正常 |
| 2. Ansible 全量运行 | 20–30 分钟 | 正常 |
| 3. 装凭据、装版本、恢复数据 | 30–60 分钟（取决于数据库大小） | 切换前的最终同步期间只读 / 维护页 |
| 4. 切换隧道 DNS | 5 分钟（CNAME 在代理之后，几乎立即生效） | 正常 |
| 5. 验证 | 15 分钟 | 正常 |
| 6. 销毁旧机器 | 10 分钟 | 正常 |

计划内约 2 小时，维护页时间约 15–30 分钟。源站正被直接攻击时，旧机器可能已不可用，第 3 步只能从备份恢复（见「数据恢复」中的缺口）。

## 1. 开新 VPS、新隧道

- 规格同生产（plan 第 13 节），Debian 13；**新 IP 从未和本域名一起用过**。泄露途径能把 IP 关联到服务商或网段时，换服务商或机房。
- 服务商控制台里给 `deploy` 账号放 SSH 公钥并给 sudo；给运维账号设控制台密码（离线保存）。PTR 保持默认。
- 新机器单独一条隧道（凭据各自独立，旧机器销毁后吊销旧隧道即可）：`cloudflared tunnel create akari-prod-<日期>`。
- SSH 主机名在 `ssh.akari.cc` 与 `ssh-next.akari.cc` 之间轮流（两个都在 Access 的 SSH 应用里）：新机器用当前**没在用**的那个，下文记作 `<新 SSH 名>`。
  `cloudflared tunnel route dns <新隧道> <新 SSH 名>`。`akari.cc` 此时仍指向旧隧道。

## 2. Ansible 全量运行

在 `group_vars/production.yml` 里**先不改**，用 `-e` 指向新机器（日常运行仍走旧机器）：

```sh
cd deploy/ansible
ansible-playbook -i inventory/production.yml site.yml \
  -e ansible_host=<新 IP> -e ansible_port=22 -e ansible_ssh_common_args= \
  -e cloudflared_tunnel_id=<新 UUID> -e cloudflared_ssh_hostname=<新 SSH 名> \
  -e cloudflared_credentials_file=$HOME/.cloudflared/<新 UUID>.json
```

跑完后新机器只能经 `<新 SSH 名>` 登录，公网端口全部关闭。切换前对新机器的运行都带
`-e cloudflared_tunnel_id=<新 UUID> -e cloudflared_ssh_hostname=<新 SSH 名>`（不带 IP 与凭据参数）。

## 3. 凭据、版本与数据

**凭据**：systemd 凭据用本机密钥加密，旧机器的 `.cred` 文件在新机器上解不开，从密码管理器里的原件重新加密：

```sh
systemd-creds encrypt --name=paymaster-signer - /etc/akari/credstore/paymaster-signer.cred      # 从 stdin 粘贴
systemd-creds encrypt --name=relayer-hot-wallet - /etc/akari/credstore/relayer-hot-wallet.cred
```

`/etc/akari/*.env`（数据库口令、Cookie 密钥、API 密钥等，见 deploy/README.md）同样从密码管理器恢复，权限 `0600 root`。
只是 IP 泄露、没有入侵迹象时沿用原密钥（换 `COOKIE_SECRET` 会让所有人重新登录）。

**版本**：用 CI 签名的同一版本（`install-release install <release.tar.zst>`），与旧机器一致。

**数据恢复**：

| 数据 | 在哪里 | 迁移方式 |
| --- | --- | --- |
| PostgreSQL | 源站 | 见下 |
| 图片、证据 | R2 | 不在源站，不用迁 |
| Valkey（缓存、会话、推送） | 源站 | 不迁；用户需要重新登录一次 |
| 链上状态 | 区块链 | indexer 从数据库里的游标继续补拉 |

PostgreSQL 的现状与缺口：

- **缺口**：pgBackRest 归档到 R2 尚未启用（等 R2 凭据，见 deploy/README.md「尚未完成」与 postgresql 角色里注释掉的 `archive_command`）。
  启用之前**没有异地备份**，只能从仍在运行的旧机器迁移；旧机器已经不可用时会丢数据（链上资金不受影响，订单与托管状态可由 indexer 从链上重建，
  帖子、私信、站内数据会丢失）。启用 pgBackRest 是上线前置条件（plan 第 16 节「备份恢复演练成功」）。
- 启用 pgBackRest 后（推荐路径）：新机器配置同一 stanza 的 R2 仓库，`pgbackrest --stanza=akari --delta restore`，切换前在旧机器上
  `pgbackrest --stanza=akari backup --type=incr` 再恢复到最新 WAL，数据差几秒。
- 目前（旧机器可用时）：网站切维护页，停写入的服务（`forum-api@*`、`forum-worker`、`forum-indexer`、`forum-sponsor`），经运维本机中转：

  ```sh
  ssh <运维账号>@<旧 SSH 名> 'sudo -u postgres pg_dump -Fc akari' > akari.dump
  scp akari.dump <运维账号>@<新 SSH 名>:/tmp/
  ssh <运维账号>@<新 SSH 名> 'sudo -u postgres pg_restore -d akari --no-owner /tmp/akari.dump && rm /tmp/akari.dump'
  ```

  （`ssh` 需要 `-o ProxyCommand="cloudflared access ssh --hostname %h"`；dump 含用户数据，传完立即删除本地副本。）数据库较大时用 `pg_basebackup` 代替。

切换前在新机器本地验证：`curl -H 'Host: akari.cc' http://127.0.0.1:8080/api/v1/healthz`，并看各服务日志。

## 4. 切换

```sh
cloudflared tunnel route dns --overwrite-dns <新隧道> akari.cc
```

然后把 `group_vars/production.yml` 的 `cloudflared_tunnel_id`、`cloudflared_ssh_hostname` 改成新 UUID 与 `<新 SSH 名>` 并合并（PR），
之后不带 `-e` 跑 playbook（ansible 经 `<新 SSH 名>` 连新机器）。
**不要**把旧隧道凭据装到新机器上：同一隧道的两个连接器会被 Cloudflare 同时分流，请求会落到两个不同的数据库上。

## 5. 验证

- 从外部主机：`deploy/check-origin-exposure.sh <新 IP> akari.cc`（有 IPv6 再跑一次）全部通过；
- 网站登录、发帖、下单页、WebSocket 推送正常；`forum-indexer` 追上最新区块；发一封测试邮件；
- `cloudflared tunnel info <旧隧道>` 已没有流量。

## 6. 销毁旧机器

1. 停旧机器上的 `cloudflared@*`，`cloudflared tunnel delete <旧隧道>`（旧凭据随即失效），删除 `<旧 SSH 名>` 的 DNS 记录（下次轮换再建）；
2. 服务商控制台销毁 VM 和磁盘、快照（支持的话先安全擦除）；
3. 旧 IP 记进运维记录：以后不再使用，也不要出现在任何 DNS 记录里；
4. 在 docs/PROGRESS.md 记一笔：泄露原因、修复、用时。

## 演练

上线前在测试服务器上完整演练一次并记录实际用时，之后每季度一次（与备份恢复演练一起）。
