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
  hexToBigInt,
  hexToBytes,
  isHex,
  keccak256,
  type Hex,
  type SignableMessage,
} from "viem";
import type { PasskeyView, Verified, WalletApi } from "./api";
import { RP_ID } from "./config";

/** 开通交易：后端出 challenge → 浏览器创建 Passkey → 后端验证并保存（plan 第 5 节第 1–2 步）。 */
export async function registerPasskey(api: WalletApi): Promise<PasskeyView> {
  const optionsJSON = await api.registrationOptions();
  return api.finishRegistration(await startRegistration({ optionsJSON }));
}

/** 再验证留的余量：截止前 30 秒内视为已过期，避免请求路上刚好失效。 */
const RECENT_MARGIN_SECONDS = 30;

/** Passkey 再验证（一次 Face ID）：会话获得 5 分钟「近期验证」，绑定到用到的那个 Passkey。 */
export async function assertPasskey(api: WalletApi): Promise<Verified> {
  const optionsJSON = await api.assertionOptions();
  return api.finishAssertion(await startAuthentication({ optionsJSON }));
}

/** 敏感操作（发报价）前确保会话有近期 Passkey 验证；已有就不再弹 Face ID。 */
export async function ensureRecentPasskey(api: WalletApi): Promise<void> {
  const { verified_until } = await api.overview();
  if (verified_until && verified_until - Date.now() / 1000 > RECENT_MARGIN_SECONDS) return;
  await assertPasskey(api);
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

/** 后端登记的 P-256 坐标 → ZeroDev 需要的 WebAuthnKey（坐标只是公开数据，不需要 Face ID）。 */
export function toWebAuthnKey(passkey: PasskeyView): WebAuthnKey {
  const { credential_id: credentialId, pubkey_x, pubkey_y } = passkey;
  if (!isHex(pubkey_x) || !isHex(pubkey_y)) throw new Error("钱包 Passkey 公钥格式不对");
  return {
    pubX: hexToBigInt(pubkey_x),
    pubY: hexToBigInt(pubkey_y),
    authenticatorId: credentialId,
    authenticatorIdHash: keccak256(new Uint8Array(base64URLStringToBuffer(credentialId))),
    rpID: RP_ID,
    signMessageCallback: (message) => signWithPasskey(message, credentialId, RP_ID),
  };
}
