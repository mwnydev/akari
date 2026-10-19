import type { WalletInitVersion } from "@akari/contracts";
import type { Address } from "viem";
import type { WalletInitVersionView } from "./api";

/**
 * 钱包初始化参数版本（ADR 0008）。版本决定 CREATE2 地址，所以每个钱包永远按自己登记的版本推算、部署。
 *
 * 信任边界：版本参数以随前端发布的链配置为准（Cloudflare Pages，CI 发布），不采信服务器下发的参数——
 * 被入侵的服务器若能改 `paymaster` / `approveToken`，新钱包就会在 initCode 里给攻击者无限授权。
 * 服务器的 `wallet_init_versions` 只用来核对（同一版本号参数必须完全一致）和确认后端认识这个版本。
 */
export class InitVersions {
  private constructor(private readonly versions: readonly WalletInitVersion[]) {}

  static resolve(
    bundled: readonly WalletInitVersion[],
    remoteViews: readonly WalletInitVersionView[],
  ): InitVersions {
    const remote = remoteViews.map(fromView);
    for (const r of remote) {
      const b = bundled.find((v) => v.version === r.version);
      if (b && !sameVersion(b, r)) {
        throw new Error(`钱包参数（版本 ${r.version}）与服务器不一致，已停止操作，请联系客服`);
      }
    }
    return new InitVersions(bundled.filter((b) => remote.some((r) => r.version === b.version)));
  }

  /** 已登记钱包用它自己的版本；当前页面不认识的版本直接报错，绝不退回「当前」配置。 */
  get(version: number): WalletInitVersion {
    const v = this.versions.find((x) => x.version === version);
    if (!v) throw new Error(`当前页面不支持这个钱包的版本（${version}），请刷新页面后重试`);
    return v;
  }

  /** 新开通的钱包用前后端都认识的最新版本。 */
  latest(): WalletInitVersion {
    const v = this.versions.reduce<WalletInitVersion | undefined>(
      (max, x) => (!max || x.version > max.version ? x : max),
      undefined,
    );
    if (!v) throw new Error("钱包参数暂不可用，请稍后再试");
    return v;
  }

  /** 全部版本的 Paymaster：代付服务返回别的合约一律拒签。 */
  paymasters(): string[] {
    return this.versions.map((v) => v.paymaster.toLowerCase());
  }
}

/** 后端 `wallet_init_versions` 的一项（与链配置 TOML 同名的蛇形字段）→ 前端链配置的形状。 */
export function fromView(v: WalletInitVersionView): WalletInitVersion {
  return {
    version: v.version,
    kernelImpl: v.kernel_implementation as Address,
    factory: v.kernel_factory as Address,
    metaFactory: v.kernel_meta_factory as Address,
    webauthnValidator: v.webauthn_validator as Address,
    entryPoint: v.entry_point as Address,
    paymaster: v.paymaster as Address,
    approveToken: v.approve_token as Address,
    approveAmount: v.approve_amount,
  };
}

/** `fromView` 的反向：后端返回的形状（地址小写），给单元测试与端到端测试的模拟后端用。 */
export function toView(v: WalletInitVersion): WalletInitVersionView {
  return {
    version: v.version,
    kernel_implementation: v.kernelImpl.toLowerCase(),
    kernel_factory: v.factory.toLowerCase(),
    kernel_meta_factory: v.metaFactory.toLowerCase(),
    webauthn_validator: v.webauthnValidator.toLowerCase(),
    entry_point: v.entryPoint.toLowerCase(),
    paymaster: v.paymaster.toLowerCase(),
    approve_token: v.approveToken.toLowerCase(),
    approve_amount: v.approveAmount,
  };
}

function sameVersion(a: WalletInitVersion, b: WalletInitVersion): boolean {
  const keys = Object.keys(a) as (keyof WalletInitVersion)[];
  return (
    keys.length === Object.keys(b).length &&
    keys.every((k) => String(a[k]).toLowerCase() === String(b[k]).toLowerCase())
  );
}
