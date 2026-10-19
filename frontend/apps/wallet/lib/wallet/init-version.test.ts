import type { WalletInitVersion } from "@akari/contracts";
import { describe, expect, it } from "vitest";
import { InitVersions, toView as view } from "./init-version";

const v1: WalletInitVersion = {
  version: 1,
  kernelImpl: "0xd6CEDDe84be40893d153Be9d467CD6aD37875b28",
  factory: "0x2577507b78c2008Ff367261CB6285d44ba5eF2E9",
  metaFactory: "0xd703aaE79538628d27099B8c4f621bE4CCd142d5",
  webauthnValidator: "0x7ab16Ff354AcB328452F1D445b3Ddee9a91e9e69",
  entryPoint: "0x0000000071727De22E5E9d8BAf0edAc6f37da032",
  paymaster: "0x6022875397e11A32BbAEB91d80258CBF4AA6F4A8",
  approveToken: "0x8c313D0eC280837325fA5aB05AC04Be31F46AbBC",
  approveAmount: "1000000",
};
const v2: WalletInitVersion = {
  ...v1,
  version: 2,
  paymaster: "0x3333333333333333333333333333333333333333",
};

describe("钱包初始化参数版本的取用与核对", () => {
  it("新钱包用前后端都认识的最新版本；地址大小写不同不算不一致", () => {
    expect(InitVersions.resolve([v1, v2], [view(v1)]).latest()).toBe(v1);
    expect(InitVersions.resolve([v1], [view(v1), view(v2)]).latest()).toBe(v1);
    expect(InitVersions.resolve([v1, v2], [view(v1), view(v2)]).get(1)).toBe(v1);
  });

  it("服务器下发的同号版本参数被改过就整体拒绝（不采信服务器的授权对象）", () => {
    const evil = { ...view(v1), paymaster: "0x4444444444444444444444444444444444444444" };
    expect(() => InitVersions.resolve([v1], [evil])).toThrow("与服务器不一致");
    expect(() => InitVersions.resolve([v1], [{ ...view(v1), approve_amount: "1" }])).toThrow();
  });

  it("不认识的版本直接报错，绝不退回当前版本", () => {
    expect(() => InitVersions.resolve([v1], [view(v1), view(v2)]).get(2)).toThrow(
      "不支持这个钱包的版本",
    );
    expect(() => InitVersions.resolve([v1], []).latest()).toThrow();
  });

  it("所有版本的 Paymaster 都可用于代付核对", () => {
    expect(InitVersions.resolve([v1, v2], [view(v1), view(v2)]).paymasters()).toEqual([
      v1.paymaster.toLowerCase(),
      v2.paymaster.toLowerCase(),
    ]);
  });
});
