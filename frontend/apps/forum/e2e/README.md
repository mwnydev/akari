# 论坛端到端测试

```sh
docker compose -f ../../../deploy/dev/compose.yaml up -d          # PostgreSQL / Valkey / Mailpit
# 后端（另开终端，环境变量见 backend/.env.example）：
#   ALLOWED_ORIGINS=http://localhost:3000 COOKIE_SECURE=false cargo run -p forum-api
#   MAIL_FROM=noreply@akari.test SMTP_URL=smtp://127.0.0.1:1025 cargo run -p forum-worker
pnpm build
# COOKIE_SIGNING_KEY = 后端 COOKIE_SECRET 的前 32 字节（前 64 个十六进制字符）
COOKIE_SIGNING_KEY=<后端 COOKIE_SECRET 前 64 个字符> DATABASE_URL=<同后端> pnpm e2e
```

`forum.spec.ts` 会把测试账号的注册时间改早两天（绕开新用户每小时 5 帖的限频，才能造出第 2 页），
所以需要能直连 forum-api 用的数据库（`DATABASE_URL`，默认本地开发库）。
论坛不在 3000 端口时设 `FORUM_URL`（须在后端 `ALLOWED_ORIGINS` 里）。

有 HTTP 代理的环境要设 `NO_PROXY=127.0.0.1,localhost`。

## 私信、通知、图片上传

- 实时推送走同源 `/api/v1/ws`（`next start` 的 rewrite 会转发 WebSocket 升级）；需要 forum-worker 在跑（通知由它从发件箱写入并推送）。
- 图片上传：forum-api 与 forum-worker 都要配 `MEDIA_S3_*`（见 backend/.env.example）。浏览器直传存储桶，
  存储桶要允许论坛来源的 `PUT` 跨源请求（R2 / SeaweedFS 的 bucket CORS）。帖子里只保留 `https://` 的本站图片，
  所以本地要给 `MEDIA_PUBLIC_BASE` 套一层 HTTPS（如 Caddy `tls internal` 反代存储桶），测试里对应的上下文用
  `ignoreHTTPSErrors`。
- `order-chat.spec.ts`、`messaging.spec.ts` 直接往 `DATABASE_URL` 写订单与报价卡片（它们本来由 indexer 与钱包写入）。
- 登录接口按 IP 限频；测试给每个用户一个假的 `CF-Connecting-IP`（forum-api 只信任来自本机的这个头），见 `auth.ts`。

### 会话「掉线」

浏览器上下文发往**另一个** forum-api 的请求会清掉会话：`localhost` 的 Cookie 不分端口，带着本站签名的 `sid`
打到一个 Valkey 里没有这条会话的 forum-api（同一个开发用 `COOKIE_SECRET`、不同的 Valkey 库）时，tower-sessions
回 `Set-Cookie: sid=; Max-Age=0`，浏览器的会话就没了（curl 有自己的 Cookie 罐，不受影响）。所以测试里的 API 请求一律
用相对地址发到页面同源，经 Next rewrite 到同一个 forum-api。
