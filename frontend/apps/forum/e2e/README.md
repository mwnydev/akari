# 论坛端到端测试

```sh
docker compose -f ../../../deploy/dev/compose.yaml up -d          # PostgreSQL / Valkey / Mailpit
# 后端（另开终端，环境变量见 backend/.env.example）：
#   ALLOWED_ORIGINS=http://localhost:3000 COOKIE_SECURE=false cargo run -p forum-api
#   MAIL_FROM=noreply@akari.test SMTP_URL=smtp://127.0.0.1:1025 cargo run -p forum-worker
pnpm build                                                         # 论坛需与后端使用同一个 COOKIE_SECRET
COOKIE_SECRET=<同后端> pnpm e2e
```

有 HTTP 代理的环境要设 `NO_PROXY=127.0.0.1,localhost`。
