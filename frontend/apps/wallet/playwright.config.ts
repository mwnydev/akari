import { defineConfig, devices } from "@playwright/test";
import { AA_RPC_PATH, API_ORIGIN, backendPaths } from "./e2e/env";

// AA 端到端测试：先 `deploy/dev/aa.sh up` 启动本地链与 bundler（见 deploy/dev/README-aa.md）；链没起来时整组跳过。
// 钱包用 `next dev` 运行；forum-api（WALLET_E2E_API，可选）实现了 /aa/rpc 就走真实代付，
// 否则用 DEV 代付（构建产物禁止带 DEV 代付，见 next.config.ts）。
// 端口可改：同一台机器上多份检出同时跑 e2e 时互不冲突
const PORT = Number(process.env.WALLET_E2E_PORT ?? 3101);
const realSponsor = (await backendPaths()).has(AA_RPC_PATH);

export default defineConfig({
  testDir: "e2e",
  timeout: 240_000,
  expect: { timeout: 60_000 },
  retries: 0,
  workers: 1,
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "retain-on-failure",
  },
  // CDP 虚拟 WebAuthn 认证器只有 Chromium 支持
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `next dev --port ${PORT}`,
    url: `http://localhost:${PORT}/setup`,
    env: {
      NEXT_PUBLIC_DEV_SPONSOR: realSponsor ? "0" : "1",
      NEXT_PUBLIC_WALLET_RP_ID: "localhost",
      API_ORIGIN,
    },
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
