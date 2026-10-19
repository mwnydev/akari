import {
  base64URLStringToBuffer,
  bufferToBase64URLString,
  startAuthentication,
  startRegistration,
} from "@simplewebauthn/browser";
import { findQuoteIndices, parseAndNormalizeSig, type WebAuthnKey } from "@zerodev/webauthn-key";
import {
  bytesToHex,
  encodeAbiParameters,
  hexToBytes,
  isHex,
  keccak256,
  type Hex,
  type SignableMessage,
} from "viem";
import type { StoredPasskey, WalletApi } from "./api";

/** 开通交易：后端出 challenge → 浏览器创建 Passkey → 后端验证并保存（plan 第 5 节第 1–2 步）。 */
export async function registerPasskey(api: WalletApi): Promise<StoredPasskey> {
  const optionsJSON = await api.registrationOptions();
  const response = await startRegistration({ optionsJSON });
  return api.verifyRegistration(response);
}

/** 待签内容 → WebAuthn challenge 字节。Kernel 签 UserOperation 与 EIP-712 时传进来的都是 32 字节哈希。 */
function challengeBytes(message: SignableMessage): Uint8Array {
  if (typeof message === "string") {
    if (!isHex(message)) throw new Error("待签内容必须是十六进制哈希");
    return hexToBytes(message);
  }
  return typeof message.raw === "string" ? hexToBytes(message.raw) : message.raw;
}

/**
 * 用 Passkey 签一个哈希，编码成 ZeroDev WebAuthn 验证器要的签名格式。
 * 自己调 @simplewebauthn/browser（版本按 plan 锁定），编码沿用 @zerodev/webauthn-key 的工具函数。
 * usePrecompiled = true：Arbitrum One / Sepolia（RIP-7212）与本地 anvil（Osaka，EIP-7951）在 0x100 都有 P-256 预编译。
 */
async function signWithPasskey(
  message: SignableMessage,
  credentialId: string,
  rpId: string,
): Promise<Hex> {
  const { response } = await startAuthentication({
    optionsJSON: {
      challenge: bufferToBase64URLString(challengeBytes(message).slice().buffer),
      allowCredentials: [{ id: credentialId, type: "public-key" }],
      userVerification: "required",
      rpId,
    },
  });
  const clientDataJSON = new TextDecoder().decode(base64URLStringToBuffer(response.clientDataJSON));
  const { r, s } = parseAndNormalizeSig(
    bytesToHex(new Uint8Array(base64URLStringToBuffer(response.signature))),
  );
  return encodeAbiParameters(
    [
      { name: "authenticatorData", type: "bytes" },
      { name: "clientDataJSON", type: "string" },
      { name: "responseTypeLocation", type: "uint256" },
      { name: "r", type: "uint256" },
      { name: "s", type: "uint256" },
      { name: "usePrecompiled", type: "bool" },
    ],
    [
      bytesToHex(new Uint8Array(base64URLStringToBuffer(response.authenticatorData))),
      clientDataJSON,
      findQuoteIndices(clientDataJSON).beforeType,
      r,
      s,
      true,
    ],
  );
}

/** 登记的公钥（SPKI DER）→ ZeroDev 需要的 P-256 坐标。坐标只是公开数据，解析交给 WebCrypto。 */
export async function toWebAuthnKey(passkey: StoredPasskey): Promise<WebAuthnKey> {
  const key = await crypto.subtle.importKey(
    "spki",
    base64URLStringToBuffer(passkey.publicKey),
    { name: "ECDSA", namedCurve: "P-256" },
    true,
    ["verify"],
  );
  const raw = new Uint8Array(await crypto.subtle.exportKey("raw", key)); // 0x04 ‖ x ‖ y
  return {
    pubX: BigInt(bytesToHex(raw.slice(1, 33))),
    pubY: BigInt(bytesToHex(raw.slice(33, 65))),
    authenticatorId: passkey.credentialId,
    authenticatorIdHash: keccak256(new Uint8Array(base64URLStringToBuffer(passkey.credentialId))),
    rpID: passkey.rpId,
    signMessageCallback: (message) => signWithPasskey(message, passkey.credentialId, passkey.rpId),
  };
}
