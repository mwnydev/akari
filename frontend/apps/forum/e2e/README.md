# 论坛端到端测试

```sh
docker compose -f ../../../deploy/dev/compose.yaml up -d          # PostgreSQL / Valkey / Mailpit
# 后端（另开终端，环境变量见 backend/.env.example）：
#   ALLOWED_ORIGINS=http://localhost:3000 COOKIE_SECURE=false cargo run -p forum-api
#   MAIL_FROM=noreply@akari.test SMTP_URL=smtp://127.0.0.1:1025 cargo run -p forum-worker
pnpm build                                                         # 论坛需与后端使用同一个 COOKIE_SECRET
COOKIE_SECRET=<同后端> DATABASE_URL=<同后端> pnpm e2e
```

`forum.spec.ts` 会把测试账号的注册时间改早两天（绕开新用户每小时 5 帖的限频，才能造出第 2 页），
所以需要能直连 forum-api 用的数据库（`DATABASE_URL`，默认本地开发库）。
论坛不在 3000 端口时设 `FORUM_URL`（须在后端 `ALLOWED_ORIGINS` 里）。

有 HTTP 代理的环境要设 `NO_PROXY=127.0.0.1,localhost`。
