import { passkeyCeremony, passkeyOptions } from "@akari/api-client";
import {
  startAuthentication,
  startRegistration,
  type PublicKeyCredentialCreationOptionsJSON,
  type PublicKeyCredentialRequestOptionsJSON,
} from "@simplewebauthn/browser";
import { api, unwrap, type AdminSessionView } from "./api";
import { ADMIN_RP_ID } from "./config";

// 后台 Passkey 的三种仪式（RP ID 为后台中心域名，ADR 0012、0013）。断言与注册结果原样交给服务端校验。

type Body = Record<string, unknown>;

function assert(options: unknown): Promise<Body> {
  return passkeyCeremony(async () => {
    const optionsJSON = passkeyOptions<PublicKeyCredentialRequestOptionsJSON>(options, ADMIN_RP_ID);
    return (await startAuthentication({ optionsJSON })) as unknown as Body;
  });
}

/** 登录后台：论坛会话里的站长账号 + 后台 Passkey。 */
export async function loginWithPasskey(): Promise<AdminSessionView> {
  const { options } = await unwrap(api.POST("/api/v1/admin/login/start"));
  const credential = await assert(options);
  return unwrap(api.POST("/api/v1/admin/login/finish", { body: { credential } }));
}

/** 敏感操作前再验证：成功后 5 分钟内可连续处理。 */
export async function reverifyWithPasskey(): Promise<AdminSessionView> {
  const { options } = await unwrap(api.POST("/api/v1/admin/reverify/start"));
  const credential = await assert(options);
  return unwrap(api.POST("/api/v1/admin/reverify/finish", { body: { credential } }));
}

/**
 * 绑定后台 Passkey：第一个要先做过邮箱验证（`enroll/verify`），成功即登录后台；
 * 已登录后台时是再加一个（要求 5 分钟内再验证过）。
 */
export async function registerPasskey(): Promise<void> {
  const { options } = await unwrap(api.POST("/api/v1/admin/passkeys/register/start"));
  const credential = await passkeyCeremony(async () => {
    const optionsJSON = passkeyOptions<PublicKeyCredentialCreationOptionsJSON>(
      options,
      ADMIN_RP_ID,
    );
    return (await startRegistration({ optionsJSON })) as unknown as Body;
  });
  await unwrap(api.POST("/api/v1/admin/passkeys/register/finish", { body: { credential } }));
}
