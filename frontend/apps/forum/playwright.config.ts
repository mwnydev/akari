import { defineConfig, devices } from "@playwright/test";

// 端到端测试依赖本地运行的 forum-api、forum-worker、Mailpit（deploy/dev/compose.yaml），见 e2e/README.md。
// 论坛本身若没在跑，会先构建好的产物上 `next start`。
const FORUM_URL = process.env.FORUM_URL ?? "http://localhost:3000";

export default defineConfig({
  testDir: "e2e",
  timeout: 30_000,
  retries: 0,
  use: {
    // 必须用 ALLOWED_ORIGINS 里登记的源，否则后端拒绝写请求
    baseURL: FORUM_URL,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "mobile", use: { ...devices["Pixel 7"] } },
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
  ],
  webServer: {
    command: `pnpm exec next start --port ${new URL(FORUM_URL).port || "3000"}`,
    url: FORUM_URL,
    reuseExistingServer: true,
    timeout: 60_000,
  },
});
