# 源站 IP 保护

目标：任何人都查不到、也连不上源站 IP；万一泄露，几小时内换掉（[origin-ip-rotation.md](origin-ip-rotation.md)）。
Cloudflare 的防 DDoS 只在流量经过它时有效，源站 IP 一旦公开，攻击者可以绕过 Cloudflare 直接打源站（plan 第 11 节）。

## 源站 IP 常见的泄露途径

| 途径 | 措施 |
| --- | --- |
| 源站开着端口，被 Censys / Shodan 扫到并按证书、页面内容关联到域名 | 公网一个端口都不开（tunnel 模式）；`check-origin-exposure.sh` 每次部署后检查 |
| DNS：没开代理的 A/AAAA、`direct.` / `origin.` 一类名字、MX 指向源站、SPF 里写源站 IP、DNS 历史记录 | 见下方「DNS 与 Cloudflare 清单」；上线用从未公开过的新 IP |
| 邮件头：SMTP 提交服务器把客户端 IP 写进 `Received` | 生产只用 HTTP API 发信（见「邮件」） |
| 源站主动连外部：抓取用户给的网址（链接预览、远程图片、webhook）会把 IP 告诉对方 | 永远不抓取用户提供的网址；出站只连固定服务（见「出站」） |
| 反向解析（PTR）写了站点域名；测试环境与生产同 IP | PTR 保持服务商默认；测试服务器单独一台、单独 IP |

## 架构决定

### 入站：网页和 SSH 都只经 Cloudflare Tunnel（`origin_ingress: tunnel`）

源站上的 `cloudflared` 主动向 Cloudflare 边缘建立出站连接，网页请求和 SSH 都从这条连接进来。防火墙入站不放行任何端口，
Caddy、sshd 只听 127.0.0.1，就算防火墙规则被清空，公网上也没有可以连的服务。

评估过保留「443 只放行 Cloudflare 网段 + Authenticated Origin Pulls」（`origin_ingress: direct`，作为备用方案保留），没有选它作为默认：

- 443 开着时，TLS 1.3 下 Caddy 在校验客户端证书**之前**就把源站证书（含 `akari.cc`）发给了对方（本地实测）。现在靠防火墙只放行 Cloudflare 网段挡住，
  但防火墙一旦配错，扫描器就能把 IP 和域名对上；tunnel 模式下根本没有端口可扫。
- 更换源站 IP 不用改 DNS 指向 IP：DNS 指向隧道（`<隧道 UUID>.cfargotunnel.com`），换机器只是换隧道，Cloudflare 后台里也不出现源站 IP。
- 不用维护 Cloudflare IP 网段列表，也不用为攻击者从其他 Cloudflare 账号回源（AOP 证书是全 Cloudflare 共用的，除非换成自有 CA）操心。
- 代价：`cloudflared` 成了网页和 SSH 的共同入口。缓解：同一隧道跑两个连接器（`cloudflared_replicas`），`Restart=always`，滚动重启；
  Cloudflare 隧道服务本身故障时，用服务商控制台应急（见「应急」）。性能上隧道走 QUIC，单连接器即可承载本项目流量，Caddy 的并发限制照旧生效。

真实客户端 IP：边缘照常带 `CF-Connecting-IP`；tunnel 模式 Caddy 只信任本机（cloudflared）传来的这个头，forum-api 只信任本机（Caddy），链条不变。
未知 Host（含用 IP 访问）Caddy 直接断开连接（`abort`），不返回任何内容。

### SSH：Cloudflare Access + 隧道，sshd 只听本机

- `cloudflared_ssh_hostname`（生产 `ssh.akari.cc`）→ 隧道 → `ssh://127.0.0.1:22022`。边缘要求 Cloudflare Access 登录；
  连接器再校验 Access JWT（`originRequest.access.required`），即使 Access 应用被误删也进不来。sshd 仍然只允许密钥。
- 公网 SSH 默认关闭。`firewall_ssh_bootstrap_allow`（默认空）可临时放行指定来源网段，只在隧道不能用时经 `-e` 传入，playbook 会打印醒目警告；
  不允许 `0.0.0.0/0`、`::/0`。放行期间 sshd 也照常监听公网。
- 不会把自己锁在外面：playbook 先装好隧道、确认每个连接器都连上边缘（`/ready`），并且**从控制机经隧道实际登录一次**（主机密钥取自当前连接并固定），
  之后才让 sshd 只听本机；公网 SSH 的防火墙规则在下一个角色（firewall）才去掉，生效在 play 结束时。任何一步失败 play 就停在关闭公网 SSH 之前。
- 应急入口是 VPS 服务商的网页控制台（VNC / 串口）。要求：运维账号设好长随机密码（只用于控制台，sshd 不接受密码），离线保存。

### 出站：按端口放行 + 代码审查

源站要连的服务（RPC、R2、Turnstile、Bundler、邮件 API、Telegram、GitHub、Cloudflare、软件源）都在 CDN 或云上，IP 经常变，
按主机名写 nftables 白名单需要定时解析、更新，漏一次就是故障，维护不了。所以：

- nftables 出站策略按**端口**放行：443、DNS（53）、DHCP、NTP（123）、隧道（7844 TCP/UDP），apt 账号可用 80；其他一律拒绝并记日志
  （`journalctl -k | grep "akari egress blocked"`）。SMTP 端口（25/465/587）因此在生产不可用。变量：`firewall_egress_enforce`、`firewall_egress_extra_tcp_ports`。
- 连哪里由代码决定：`backend/crates/runtime/tests/outbound_clients.rs` 扫描所有源码，新建出站客户端（reqwest、alloy provider、SMTP、Resend……）
  的地方必须登记在白名单里，新增就让 CI 失败，由审查确认目标是运维配置的固定服务。
- 规则：**源站永远不抓取用户提供的网址**（链接预览、远程图片、头像网址、用户填的 webhook 都不做）。以后确实需要时，经不在源站上的独立出网代理
  （如 Cloudflare Worker）转发，源站只和代理通信。论坛前端（Next.js 服务端）同样适用：服务端渲染只请求本站 API。

### 邮件：生产只用 HTTP API

实测（Mailpit，2026-10-19）：SMTP 提交后服务器加上 `Received: from DESKTOP-J7URVD6 (unknown [172.19.0.1])`——EHLO 名是本机主机名，方括号里是客户端 IP。
真实服务商的 SMTP 提交服务器通常也这样记录（Received 头是 RFC 5321 要求的），源站 IP 会出现在每封邮件头里。HTTP API 提交时邮件由服务商生成，
`Received` 链从服务商自己的服务器开始，不含调用方 IP。

- 主备两个 HTTP API 通道：Resend（主）与 Postmark（备，`forum_mail::PostmarkMailer`）。两家各自的发信基础设施，一家故障或封号不影响另一家；Postmark 的 API 只需一个令牌请求头，不用 SigV4 签名，复用现有 reqwest，不新增依赖。
- SMTP 只用于开发：`SmtpMailer` 只接受本机地址（Mailpit），出站防火墙也不放行 SMTP 端口。即使在本机，EHLO 与 Message-ID 也用发件域名
  （`<随机 32 位十六进制@akari.cc>`），不用主机名；`forum-mail` 的测试锁定发出的邮件里没有 `Received`、`X-Originating-IP`、`X-Mailer` 和本机主机名，
  Postmark 请求里没有自定义邮件头、不开打开与点击追踪。
- **待接入**：forum-worker 的 `build_mailer`（`bins/worker/src/main.rs`）还没有 `postmark` 分支，需要加
  `"postmark" => AnyMailer::Postmark(PostmarkMailer::new(&env("POSTMARK_SERVER_TOKEN")?, from)?)`；生产配置 `MAIL_PRIMARY=resend`、`MAIL_BACKUP=postmark`。

## 首次初始化（新服务器）

站长先在 Cloudflare 完成（只做一次，换服务器时只需第 2 步）：

1. Zero Trust：建团队（团队名填 `cloudflared_access_team`）；Access → 应用 → 自托管，域名 `ssh.akari.cc` 与 `ssh-next.akari.cc`（换服务器时轮流使用），
   策略只允许运维的邮箱 + 硬件密钥 / Passkey；把应用的 AUD 填进 `cloudflared_ssh_access_aud`。
2. 建隧道（每台服务器一条）：`cloudflared tunnel login` 后 `cloudflared tunnel create akari-prod-<日期>`，UUID 填 `cloudflared_tunnel_id`；
   生成的 `~/.cloudflared/<UUID>.json` 是隧道凭据，只放运维本机和密码管理器，**不进仓库**。
3. DNS：`cloudflared tunnel route dns <隧道> akari.cc` 和 `... ssh.akari.cc`（生成代理的 CNAME，指向隧道，不出现 IP）。
4. CI 自动部署要登录 SSH 时，建 Access 服务令牌并加进 SSH 应用策略，CI 里设 `TUNNEL_SERVICE_TOKEN_ID` / `TUNNEL_SERVICE_TOKEN_SECRET`。

运维本机装好与服务器同版本的 `cloudflared`，`cloudflared access login https://ssh.akari.cc` 登录一次，然后第一次经公网 IP 跑 playbook：

```sh
cd deploy/ansible
ansible-playbook -i inventory/production.yml site.yml \
  -e ansible_host=<源站 IP> -e ansible_port=22 -e ansible_ssh_common_args= \
  -e cloudflared_credentials_file=$HOME/.cloudflared/<UUID>.json
```

- `ansible_port=22`：全新镜像的 sshd 还在默认端口；base 角色把它改到 22022 后（play 中途就生效），失败重跑时去掉这个参数。
- 凭据经 stdin 交给 `systemd-creds encrypt`（绑定本机密钥）存为 `/etc/akari/credstore/cloudflared-tunnel.cred`，明文不落服务器磁盘、不进 Ansible 日志。
- play 结束时公网 SSH 已关闭。之后日常运行不带任何 `-e`，经隧道连接：`ansible-playbook -i inventory/production.yml site.yml`。
- 想在初始化期间保留公网 SSH：加 `-e '{"firewall_ssh_bootstrap_allow": ["<你的出口 IP>/32"]}'`，完成后不带它重跑一次关闭。
- 最后从另一台外部主机跑暴露检查（见下）。

## 应急：隧道不可用

1. 先看是不是 Cloudflare 隧道服务本身故障（cloudflarestatus.com）；连接器进程问题通常两个连接器不会同时出现。
2. 用服务商控制台登录，看 `systemctl status 'cloudflared@*'`、`journalctl -u cloudflared@1`。
3. 需要临时公网 SSH 时，在控制台里执行（重启或重跑 playbook 后失效）：
   `nft insert rule inet filter input ip saddr <你的出口 IP> tcp dport 22022 accept`，并
   `rm /etc/ssh/sshd_config.d/20-akari-listen.conf && systemctl restart ssh`；然后带 `firewall_ssh_bootstrap_allow` 跑 playbook 修复，修好后不带它再跑一次。
4. 隧道长时间不可用而网站必须恢复：`-e origin_ingress=direct` 跑 playbook（443 只放行 Cloudflare 网段 + AOP），需要 Origin CA 证书与回源 CA 已放在
   `/etc/caddy/certs/`，DNS 把 `akari.cc` 改成指向源站 IP 的**代理** A/AAAA 记录。这会把 IP 交给 Cloudflare DNS（不公开），隧道恢复后改回并考虑换 IP。

## DNS 与 Cloudflare 清单（上线前、每次改 DNS 后）

- [ ] 所有记录都是代理（橙色云）；没有任何不代理的 A/AAAA，包括测试、临时、`direct.`、`origin.`、`cpanel.`、`ftp.` 一类名字
- [ ] `akari.cc`、`ssh.akari.cc` 是指向 `<UUID>.cfargotunnel.com` 的代理 CNAME；钱包、后台在 Cloudflare Pages
- [ ] MX 指向第三方邮件服务（不是源站）；SPF 只有服务商的 `include:`，没有 `ip4:` / `ip6:` 源站地址；DKIM、DMARC 生效
- [ ] 上线用的源站 IP 从未和本域名一起公开过（没在旧 DNS 记录、旧证书、邮件头、测试环境里出现）；PTR 保持服务商默认，不写站点域名
- [ ] SSL/TLS「Always Use HTTPS」开启，HSTS 开启；direct 模式时 SSL 模式为 Full (strict) 且开启 Authenticated Origin Pulls
- [ ] Cloudflare 账号开启双重验证；只有站长和必要的运维有权限（DNS 修改权限 = 能把流量指走）
- [ ] 外部主机跑 `deploy/check-origin-exposure.sh` 全部通过（IPv4、IPv6 各一次）

## 暴露检查

```sh
deploy/check-origin-exposure.sh <源站 IP> akari.cc
```

在**外部主机**上跑（不是源站、不经 WARP）：TCP 22/80/443/5432/6379/7844/8080/22022 等必须关闭或无响应；直接用 IP 访问 HTTPS（带或不带 SNI）、
HTTP 带 Host 都不能拿到响应，TLS 也不能送出任何证书；公共 DNS（1.1.1.1、8.8.8.8、9.9.9.9）里域名、常见子域名、MX 主机、TXT 都不含源站 IP；
PTR 不含站点域名。`PORTS`、`SUBDOMAINS`、`RESOLVERS` 可用环境变量覆盖。

每次部署、改防火墙或 DNS、更换源站 IP 后都要跑，有失败就当作 IP 可能已泄露处理（修复原因，必要时按 [origin-ip-rotation.md](origin-ip-rotation.md) 换 IP）。

## Cloudflare 网段刷新（只用于 direct 模式）

tunnel 模式不需要。direct 模式下防火墙和 Caddy `trusted_proxies` 用的网段每次跑 playbook 时从 cloudflare.com 拉取；只刷新网段：

```sh
ansible-playbook -i inventory/production.yml site.yml --tags cloudflare-ips
```

Cloudflare 改网段会提前公告（cloudflare.com/ips），至少每月跑一次。
