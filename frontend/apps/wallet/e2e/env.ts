import { localChain as chain } from "@akari/contracts";
import { createPublicClient, http } from "viem";

// 端到端测试的运行环境探测：本地 AA 链（deploy/dev/aa.sh）与 forum-api（可选）。
// playwright.config.ts 与测试文件共用，结果一致。

/** forum-api 源站；`next dev` 的 /api/v1 rewrite 也指向它。 */
export const API_ORIGIN = process.env.WALLET_E2E_API ?? "http://127.0.0.1:8080";
export const AA_RPC_PATH = "/api/v1/aa/rpc";

/**
 * 链与 bundler 的地址。默认是 chain.local.json 里的默认端口；同一台机器上另有检出占着默认端口时，
 * 用 aa.sh 的 ANVIL_PORT / BUNDLER_PORT 起自己的链，再设这两个变量（浏览器里的请求由 harness 转过去）。
 */
export const RPC_URL = process.env.WALLET_E2E_RPC_URL ?? chain.rpcUrl;
export const BUNDLER_URL = process.env.WALLET_E2E_BUNDLER_URL ?? chain.bundlerUrl;

export const publicClient = createPublicClient({ transport: http(RPC_URL) });

/** 链与 bundler 都在线返回 null，否则返回跳过原因。 */
export async function chainIsDown(): Promise<string | null> {
  try {
    if ((await publicClient.getChainId()) !== chain.chainId)
      return "anvil 的链 ID 与 chain.local.json 不符";
    if (!(await publicClient.getCode({ address: chain.contracts.Escrow }))) {
      return "Escrow 没有部署（chain.local.json 与当前链不符，重新运行 aa.sh up）";
    }
    const res = await fetch(BUNDLER_URL, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "eth_supportedEntryPoints",
        params: [],
      }),
    });
    if (!res.ok) return "bundler 没有响应";
    return null;
  } catch {
    return "本地 AA 开发链没有启动（deploy/dev/aa.sh up）";
  }
}

/** 在线的 forum-api 实现了哪些接口（OpenAPI 路径模板）；没有运行时为空集合。 */
export async function backendPaths(): Promise<Set<string>> {
  try {
    const res = await fetch(`${API_ORIGIN}/api/v1/openapi.json`);
    if (!res.ok) return new Set();
    const spec = (await res.json()) as { paths?: Record<string, unknown> };
    return new Set(Object.keys(spec.paths ?? {}));
  } catch {
    return new Set();
  }
}
