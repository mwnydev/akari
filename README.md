# akari

论坛 + 链上 USDT 担保交易（Arbitrum One）。完整开发计划见 [docs/plan-v3.md](docs/plan-v3.md)，当前进度见 [docs/PROGRESS.md](docs/PROGRESS.md)。

| 目录 | 内容 |
| --- | --- |
| `backend/` | Rust Cargo workspace：`api`、`worker`、`indexer`、`sponsor` 四个程序 |
| `frontend/` | pnpm + Turborepo：`forum`（SSR）、`wallet`、`admin`（静态导出） |
| `contracts/` | Foundry：`Escrow`、`StreamEscrow` |
| `test-vectors/` | 费用 / 分期解锁共享测试向量，合约、Rust、TS 三边测试共用 |
| `docs/adr/` | 技术决策记录 |
| `deploy/` | systemd、Caddyfile、Ansible |

## 本地开发环境

Rust 1.99.1（`rust-toolchain.toml` 自动选择）、Node 24.22.0、pnpm 12.11.2、Foundry 1.8.3、Docker（PostgreSQL 18 / Valkey 9 / Mailpit）。

```sh
docker compose -f deploy/dev/compose.yaml up -d   # 本地依赖
cd backend && cargo nextest run
cd frontend && pnpm install && pnpm turbo run build
cd contracts && forge test
```

缺少测试前提（anvil / forge、Valkey、S3 兼容存储、合约产物）的后端测试默认打印 `skipped: …` 跳过；`AKARI_TEST_STRICT=1`（CI 设置）时直接失败，确认全部真跑过再交付。

## 本地预检（lefthook）

CI 是唯一的硬门禁；`lefthook.yml` 只做秒级检查（改动文件的 rustfmt、prettier、eslint、forge fmt，装了 gitleaks 时再扫密钥）。lefthook 在 `frontend` 的 devDependencies 里（版本锁定），`pnpm install` 不会自动装 git 钩子。

jj 不跑 git 钩子，按当前改动（`@`）手动运行；可在 `.jj/repo/config.toml` 里加别名，之后 `jj precheck`：

```toml
[aliases]
precheck = ["util", "exec", "--", "bash", "-c", """
cd "$JJ_WORKSPACE_ROOT" && jj diff --name-only | while read -r f; do [ -e "$f" ] && echo "--file=$f"; done \
  | xargs frontend/node_modules/.bin/lefthook run pre-commit"""]
```

用 git 提交时 `frontend/node_modules/.bin/lefthook install` 装上钩子即可。较重的检查（clippy、前端 lint/typecheck/test、knip、jscpd）默认关闭：`LEFTHOOK_HEAVY=1 frontend/node_modules/.bin/lefthook run pre-push`。单独跑 knip / jscpd：`cd frontend && pnpm knip && pnpm dup`。
