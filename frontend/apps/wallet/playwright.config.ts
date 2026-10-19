import { defineConfig, devices } from "@playwright/test";

// AA 端到端测试：先 `deploy/dev/aa.sh up` 启动本地链与 bundler（见 deploy/dev/README-aa.md）；链没起来时整组跳过。
// 钱包用 `next dev` + DEV 代付运行（构建产物禁止带 DEV 代付，见 next.config.ts）。
const PORT = 3101;

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
    env: { NEXT_PUBLIC_DEV_SPONSOR: "1", NEXT_PUBLIC_WALLET_RP_ID: "localhost" },
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
