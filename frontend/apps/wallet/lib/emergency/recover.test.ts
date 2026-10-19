import { p256 } from "@noble/curves/nist";
import { toHex } from "viem";
import { describe, expect, it } from "vitest";
import type { Assertion } from "../wallet/webauthn";
import { publicKeyCandidates, recoverPasskeyKey, signedHash } from "./recover";

// 用真实的 P-256 签名模拟 WebAuthn 断言：同一把私钥签两次不同的 challenge，应当恰好找回这把公钥。

const priv = p256.utils.randomPrivateKey();
const pub = p256.getPublicKey(priv, false);

function assertion(challenge: string, credentialId = "cred-1"): Assertion {
  const a: Assertion = {
    credentialId,
    authenticatorData: crypto.getRandomValues(new Uint8Array(37)),
    clientDataJSON: JSON.stringify({ type: "webauthn.get", challenge, origin: "https://w.test" }),
    signature: "0x",
  };
  const sig = p256.sign(signedHash(a), priv, { prehash: false, lowS: false });
  return { ...a, signature: `0x${sig.toDERHex()}` };
}

describe("从两次 Passkey 签名找回公钥", () => {
  it("一次签名的候选里包含真实公钥", () => {
    const candidates = publicKeyCandidates(assertion("a")).map((k) => toHex(k));
    expect(candidates).toContain(toHex(pub));
  });

  it("第二次签名排除错误候选，只剩真实公钥", () => {
    const key = recoverPasskeyKey(assertion("a"), assertion("b"));
    expect(toHex(key.pubX, { size: 32 })).toBe(toHex(pub.slice(1, 33)));
    expect(toHex(key.pubY, { size: 32 })).toBe(toHex(pub.slice(33)));
    expect(key.credentialId).toBe("cred-1");
  });

  it("两次用了不同的 Passkey 时报错", () => {
    expect(() => recoverPasskeyKey(assertion("a"), assertion("b", "cred-2"))).toThrow(
      "同一个 Passkey",
    );
  });

  it("签名被篡改时找不到公钥", () => {
    const second = assertion("b");
    const tampered = { ...second, clientDataJSON: second.clientDataJSON.replace("b", "c") };
    expect(() => recoverPasskeyKey(assertion("a"), tampered)).toThrow("没能从签名里算出公钥");
  });
});
