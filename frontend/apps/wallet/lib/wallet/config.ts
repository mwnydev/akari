import { localChain, type ChainConfig } from "@akari/contracts";

// 构建期常量（Next 把 NEXT_PUBLIC_* 内联进产物）。本文件很轻，可以被首屏代码引用；钱包 SDK 在 ./sdk.ts，只能动态加载。

/** 当前只有本地 AA 开发链（deploy/dev/aa.sh）；Arbitrum Sepolia / One 的配置接入时按 NEXT_PUBLIC_WALLET_CHAIN 选择。 */
export const chain: ChainConfig = localChain;

/** 钱包 Passkey 的 RP ID = 钱包中心域名（plan 第 5 节），绝不能是论坛主域名。本地开发为 localhost。 */
export const RP_ID = process.env.NEXT_PUBLIC_WALLET_RP_ID ?? "localhost";

/**
 * forum-api 所在源（ADR 0009：钱包子域名跨源调用论坛 API、带论坛会话 Cookie，后端只对 WALLET_ORIGINS 开 CORS）。
 * 留空 = 同源 `/api/v1`：`next dev` 由 next.config.ts 的 rewrite 转发到本地 forum-api。
 */
export const API_BASE = process.env.NEXT_PUBLIC_API_ORIGIN ?? "";

/** 论坛站点：钱包本身不做登录，未登录时引导到论坛登录页。 */
export const FORUM_ORIGIN = process.env.NEXT_PUBLIC_FORUM_ORIGIN ?? "http://localhost:3000";

/** ERC-7677 代付 + ERC-4337 bundler 的 JSON-RPC 入口（ADR 0009），forum-api 校验会话后转给 forum-sponsor。 */
export const AA_RPC_URL = `${API_BASE}/api/v1/aa/rpc`;

/** 仅开发：用公开的开发私钥直接给 UserOperation 签代付（./dev-sponsor.ts）。`next build` 时开启会直接报错，见 next.config.ts。 */
export const DEV_SPONSOR = process.env.NEXT_PUBLIC_DEV_SPONSOR === "1";

/** 确认页、余额卡显示的网络名：主网固定「Arbitrum One」，本地链明确标出来，避免误以为在主网签名。 */
export const NETWORK_LABEL = chain.chainId === 31337 ? "本地测试链" : chain.name;
