# 0012 正式域名与 Passkey 中心域名

- 状态：已采纳（站长决定，2026-10-19）
- 日期：2026-10-19
- 解决：plan 第 17 节待决「两个 Passkey 中心域名与备用域名的具体选择」及论坛主站域名（备用域名仍待定）

## 决定

| 用途 | RP ID（中心域名） | 页面网址 | 部署 |
| --- | --- | --- | --- |
| 钱包签名 | `wallet.akari.cc` | `https://wallet.akari.cc` | Cloudflare Pages（静态导出） |
| 后台登录（仅站长，ADR 0013） | `administrator.akari.cc` | `https://administrator.akari.cc` | Cloudflare Pages（静态导出） |
| 用户上传图片（无 Passkey） | — | `https://oss.akari.cc/` | Cloudflare R2 自定义域名 |

- 钱包与后台都直接部署在各自的中心域名上，所有浏览器（含 Firefox）原生支持，不依赖「关联来源请求」。将来加备用入口域名时，再在中心域名的 `/.well-known/webauthn` 登记关联域名（每组最多 5 个主域名，CI 签名发布）。
- 两组互不包含，也都不是论坛主站的父域名：论坛主站（含被攻破或 XSS 的情况）在浏览器层面无法请求钱包签名或管理员登录凭证（plan 第 5 节）。
- 钱包与论坛同属可注册域名 `akari.cc`，ADR 0009 的同站假设成立：forum-api 的 `WALLET_ORIGINS` = `https://wallet.akari.cc`，`WALLET_RP_ID` = `wallet.akari.cc`；会话 Cookie 保持 host-only。
- 论坛主站：`https://akari.cc`（自有服务器，经 Cloudflare），写入 forum-api 的 `ALLOWED_ORIGINS`。
- 主站在顶级域名 `akari.cc` 上，会话 Cookie **必须保持 host-only，永远不能设置 `Domain=akari.cc`**：否则 Cookie 会发到所有子域名（包括钱包和后台），一处子域名被攻破就能拿到论坛会话。代码不设置 Domain，第 7 轮已加测试锁定（登录、改用户名、重新登录、退出的所有 Set-Cookie 都不带 Domain），任何改动 Cookie 属性的 PR 需两人审查。
- 图片（2026-10-19 补充）：`MEDIA_PUBLIC_BASE` = `https://oss.akari.cc/`。浏览器用预签名地址直传 R2，不经源站；公开目录只放 worker 转码后的 WebP，原图在 `tmp/`（不公开，1 天过期）。oss 是同站子域名，安全靠：会话 Cookie host-only 不会发到 oss；Cloudflare 响应头规则给 oss 加 `X-Content-Type-Options: nosniff`、`Content-Security-Policy: sandbox; default-src 'none'`、`Cross-Origin-Resource-Policy: same-site`；存储桶 CORS 只允许 `https://akari.cc` 上传（上线清单项）。
- 主站 `akari.cc` 可以请求 RP ID 为 `akari.cc` 的 Passkey，但请求不到 `wallet.akari.cc` 和 `administrator.akari.cc`；所以**钱包和后台的 Passkey 绝不能用 `akari.cc` 作 RP ID**。唯一例外是签署仲裁裁决用的论坛 Passkey（ADR 0013），它本来就只在论坛内使用。

## 后果

- `akari.cc` 是整个系统的单点：域名丢失时，P1 恢复码上线前用户无法签名。注册商须开启注册商锁与双重验证（上线清单项）。
- 这些域名只能经代码审查与人工批准的发布流程修改，不进后台在线配置（plan 第 9 节）。
