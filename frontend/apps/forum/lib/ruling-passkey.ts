// 论坛裁决 Passkey（ADR 0012、0013 第 4 条）：RP ID 是论坛主站自己的主机名，只用于签署裁决；
// 与钱包 Passkey（钱包子域名）、后台 Passkey（后台子域名）是三组互不相通的凭证。

import { passkeyCeremony, passkeyOptions, type Schemas } from "@akari/api-client";
import {
  startAuthentication,
  startRegistration,
  type PublicKeyCredentialCreationOptionsJSON,
  type PublicKeyCredentialRequestOptionsJSON,
} from "@simplewebauthn/browser";
import { api, unwrap } from "./browser-api";

type Credential = Record<string, unknown>;
export type RulingRequest = Schemas["RulingRequest"];

/** 服务端的 RULING_RP_ID 必须等于论坛页面的主机名（启动时校验），这里再核对一次。 */
function rpId(): string {
  return window.location.hostname;
}

/** 第一个裁决 Passkey 前的邮箱验证：发码。 */
export function sendEnrollCode() {
  return unwrap(api.POST("/api/v1/arbitration/passkey/reverify/start"));
}

/** 校验验证码并创建裁决 Passkey（10 分钟内、一次性）。 */
export async function enrollRulingPasskey(code: string): Promise<void> {
  await unwrap(api.POST("/api/v1/arbitration/passkey/reverify/verify", { body: { code } }));
  const { options } = await unwrap(api.POST("/api/v1/arbitration/passkey/register/start"));
  const credential = await passkeyCeremony(async () => {
    const optionsJSON = passkeyOptions<PublicKeyCredentialCreationOptionsJSON>(options, rpId());
    return (await startRegistration({ optionsJSON })) as unknown as Credential;
  });
  await unwrap(api.POST("/api/v1/arbitration/passkey/register/finish", { body: { credential } }));
}

/** 第一步：服务端校验裁决、按链上订单算金额，返回绑定这份裁决的 WebAuthn 参数与签署内容。 */
export function rulingOptions(caseId: number, body: RulingRequest) {
  return unwrap(
    api.POST("/api/v1/arbitration/cases/{id}/ruling/options", {
      params: { path: { id: caseId } },
      body,
    }),
  );
}

/** 第二步：用裁决 Passkey 签署（challenge 就是签署内容的哈希），连同同一份裁决提交。 */
export async function signAndSubmit(
  caseId: number,
  body: RulingRequest,
  options: unknown,
): Promise<Schemas["RulingSubmitted"]> {
  const credential = await passkeyCeremony(async () => {
    const optionsJSON = passkeyOptions<PublicKeyCredentialRequestOptionsJSON>(options, rpId());
    return (await startAuthentication({ optionsJSON })) as unknown as Credential;
  });
  return unwrap(
    api.POST("/api/v1/arbitration/cases/{id}/rulings", {
      params: { path: { id: caseId } },
      body: { ...body, credential },
    }),
  );
}
