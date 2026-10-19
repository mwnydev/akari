import { p256 } from "@noble/curves/nist";
import { concat, hexToBigInt, hexToBytes, sha256, stringToBytes, toHex } from "viem";
import type { Assertion, PasskeyPublicKey } from "../wallet/webauthn";

// 不靠后端找回钱包 Passkey 的公钥：钱包地址由公钥 + 凭证 ID 决定（CREATE2），而 WebAuthn 只在注册时给出公钥。
// ECDSA 可以从签名反推出公钥的几个候选，再用第二次签名验掉错误的候选，剩下的就是唯一的公钥。

/** WebAuthn 签的是 sha256(authenticatorData ‖ sha256(clientDataJSON))。 */
export function signedHash(a: Assertion): Uint8Array {
  return sha256(
    concat([a.authenticatorData, sha256(stringToBytes(a.clientDataJSON), "bytes")]),
    "bytes",
  );
}

/** 一次签名能反推出的全部公钥候选（未压缩点，0x04 开头）。 */
export function publicKeyCandidates(a: Assertion): Uint8Array[] {
  const sig = p256.Signature.fromDER(a.signature.slice(2));
  const hash = signedHash(a);
  const out: Uint8Array[] = [];
  for (let bit = 0; bit < 4; bit++) {
    try {
      out.push(sig.addRecoveryBit(bit).recoverPublicKey(hash).toRawBytes(false));
    } catch {
      // 这个恢复位不对应曲线上的点
    }
  }
  return out;
}

function verifies(a: Assertion, publicKey: Uint8Array): boolean {
  return p256.verify(hexToBytes(a.signature), signedHash(a), publicKey, {
    format: "der",
    lowS: false,
    prehash: false,
  });
}

/** 两次签名（同一个 Passkey、不同 challenge）→ 唯一的公钥。 */
export function recoverPasskeyKey(first: Assertion, second: Assertion): PasskeyPublicKey {
  if (first.credentialId !== second.credentialId) {
    throw new Error("两次用的不是同一个 Passkey，请重试并选择同一个钱包 Passkey");
  }
  const keys = publicKeyCandidates(first).filter((k) => verifies(second, k));
  const [key, ...rest] = keys;
  if (!key || rest.length > 0) throw new Error("没能从签名里算出公钥，请重试");
  return {
    credentialId: first.credentialId,
    pubX: hexToBigInt(toHex(key.slice(1, 33))),
    pubY: hexToBigInt(toHex(key.slice(33, 65))),
  };
}
