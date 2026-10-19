// 生产构建里 "akari-dev-sponsor" 解析到这里（next.config.ts）：不含任何私钥，调用即报错。
import type { SponsorClient } from "./sponsor";

export function createDevSponsorClient(): SponsorClient {
  throw new Error("DEV 代付只在本地开发可用");
}
