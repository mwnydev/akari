import {
  base64URLStringToBuffer,
  bufferToBase64URLString,
  startAuthentication,
  type AuthenticationResponseJSON,
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
import { RP_ID } from "./config";

// 钱包 Passkey 的签名与 Kernel WebAuthn 验证器的签名格式。不依赖论坛 API：紧急取回页（lib/emergency）也用这里。

/** 钱包 Passkey 的公开部分：凭证 ID（base64url）与 P-256 公钥坐标。 */
export interface PasskeyPublicKey {
  credentialId: string;
  pubX: bigint;
  pubY: bigint;
}

/** 一次 WebAuthn 断言里验签要用的三样东西（`signature` 是 DER 编码）。 */
export interface Assertion {
  credentialId: string;
  authenticatorData: Uint8Array;
  clientDataJSON: string;
  signature: Hex;
}

export function parseAssertion({ id, response }: AuthenticationResponseJSON): Assertion {
  const bytes = (b64: string) => new Uint8Array(base64URLStringToBuffer(b64));
  return {
    credentialId: id,
    authenticatorData: bytes(response.authenticatorData),
    clientDataJSON: new TextDecoder().decode(bytes(response.clientDataJSON)),
    signature: bytesToHex(bytes(response.signature)),
  };
}

/**
 * 让钱包 Passkey 签一段 challenge（一次 Face ID）。不给 `credentialId` 时由用户在系统弹窗里选 Passkey
 * （可发现凭证），紧急取回页没有后端告诉它是哪一个。
 */
export async function getAssertion(
  challenge: Uint8Array,
  credentialId?: string,
): Promise<Assertion> {
  return parseAssertion(
    await startAuthentication({
      optionsJSON: {
        challenge: bufferToBase64URLString(challenge.slice().buffer),
        allowCredentials: credentialId ? [{ id: credentialId, type: "public-key" }] : [],
        userVerification: "required",
        rpId: RP_ID,
      },
    }),
  );
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
async function signWithPasskey(message: SignableMessage, credentialId: string): Promise<Hex> {
  const a = await getAssertion(challengeBytes(message), credentialId);
  const { r, s } = parseAndNormalizeSig(a.signature);
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
      bytesToHex(a.authenticatorData),
      a.clientDataJSON,
      findQuoteIndices(a.clientDataJSON).beforeType,
      r,
      s,
      true,
    ],
  );
}

/** 公钥 → ZeroDev 需要的 WebAuthnKey；签名时才调起 Face ID。 */
export function passkeyKey({ credentialId, pubX, pubY }: PasskeyPublicKey): WebAuthnKey {
  return {
    pubX,
    pubY,
    authenticatorId: credentialId,
    authenticatorIdHash: keccak256(new Uint8Array(base64URLStringToBuffer(credentialId))),
    rpID: RP_ID,
    signMessageCallback: (message) => signWithPasskey(message, credentialId),
  };
}
