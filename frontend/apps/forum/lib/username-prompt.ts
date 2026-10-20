// 新用户「设置用户名」提示：每个账号只主动弹一次（点「以后再说」后记在本机）；交易入口每次都会再问。
// localStorage 读写都包在 try/catch 里（隐私模式、禁用存储时当作没记录，最多多问一次）。

const KEY_PREFIX = "akari:username-prompt-dismissed:";

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function promptDismissed(userId: number): boolean {
  try {
    return storage()?.getItem(`${KEY_PREFIX}${userId}`) === "1";
  } catch {
    return false;
  }
}

export function dismissPrompt(userId: number): void {
  try {
    storage()?.setItem(`${KEY_PREFIX}${userId}`, "1");
  } catch {
    // 记不住就下次再问一次
  }
}
