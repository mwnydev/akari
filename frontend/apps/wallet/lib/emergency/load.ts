// 紧急取回 SDK 的唯一入口：动态 import，viem / ZeroDev 只进按需加载的分包（plan 第 12 节）。
type EmergencySdk = typeof import("./sdk");

let pending: Promise<EmergencySdk> | undefined;

export function loadEmergencySdk(): Promise<EmergencySdk> {
  pending ??= import("./sdk").catch((e: unknown) => {
    pending = undefined; // 分包加载失败时允许重试
    throw e;
  });
  return pending;
}
