import { localChain, type ChainConfig } from "@akari/contracts";

// 构建期常量（Next 把 NEXT_PUBLIC_* 内联进产物）。本文件很轻，可以被首屏代码引用；钱包 SDK 在 ./sdk.ts，只能动态加载。

/** 当前只有本地 AA 开发链（deploy/dev/aa.sh）；Arbitrum Sepolia / One 的配置接入时按 NEXT_PUBLIC_WALLET_CHAIN 选择。 */
export const chain: ChainConfig = localChain;

/** 钱包 Passkey 的 RP ID = 钱包中心域名（plan 第 5 节），绝不能是论坛主域名。本地开发为 localhost。 */
export const RP_ID = process.env.NEXT_PUBLIC_WALLET_RP_ID ?? "localhost";
export const RP_NAME = "Akari 钱包";

/** 仅开发：用公开的开发私钥直接给 UserOperation 签代付（./dev-sponsor.ts）。`next build` 时开启会直接报错，见 next.config.ts。 */
export const DEV_SPONSOR = process.env.NEXT_PUBLIC_DEV_SPONSOR === "1";

/** 确认页、余额卡显示的网络名：主网固定「Arbitrum One」，本地链明确标出来，避免误以为在主网签名。 */
export const NETWORK_LABEL = chain.chainId === 31337 ? "本地测试链" : chain.name;
