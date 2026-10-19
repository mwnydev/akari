// 钱包 SDK 的唯一入口：动态 import，让 viem / ZeroDev 只进按需加载的分包，不进任何页面的首屏 JS（plan 第 12 节）。
type Sdk = typeof import("./sdk");

let pending: Promise<Sdk> | undefined;

export function loadWalletSdk(): Promise<Sdk> {
  pending ??= import("./sdk").catch((e: unknown) => {
    pending = undefined; // 网络抖动导致分包加载失败时允许重试
    throw e;
  });
  return pending;
}
