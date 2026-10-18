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
