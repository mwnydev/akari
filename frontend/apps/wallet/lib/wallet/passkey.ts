import { startAuthentication, startRegistration } from "@simplewebauthn/browser";
import type { WebAuthnKey } from "@zerodev/webauthn-key";
import { hexToBigInt, isHex } from "viem";
import type { PasskeyView, Verified, WalletApi } from "./api";
import { passkeyKey } from "./webauthn";

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

/** 敏感操作（发报价、改提币地址）前确保会话有近期 Passkey 验证；已有就不再弹 Face ID。 */
export async function ensureRecentPasskey(api: WalletApi): Promise<void> {
  const { verified_until } = await api.overview();
  if (verified_until && verified_until - Date.now() / 1000 > RECENT_MARGIN_SECONDS) return;
  await assertPasskey(api);
}

/** 后端登记的 P-256 坐标 → ZeroDev 需要的 WebAuthnKey（坐标只是公开数据，不需要 Face ID）。 */
export function toWebAuthnKey(passkey: PasskeyView): WebAuthnKey {
  const { credential_id: credentialId, pubkey_x, pubkey_y } = passkey;
  if (!isHex(pubkey_x) || !isHex(pubkey_y)) throw new Error("钱包 Passkey 公钥格式不对");
  return passkeyKey({ credentialId, pubX: hexToBigInt(pubkey_x), pubY: hexToBigInt(pubkey_y) });
}
