// 构建时写死的公开配置（静态导出，没有服务端）。生产值见 ADR 0012；本地开发与端到端测试用 NEXT_PUBLIC_* 覆盖。

/** forum-api 所在源站：后台页面跨源调用 `/api/v1/admin/`，带论坛主机上的 Cookie（同站）。 */
export const API_ORIGIN = process.env.NEXT_PUBLIC_API_ORIGIN ?? "https://akari.cc";

/** 论坛主站：站长先在这里用邮箱登录，后台再用后台 Passkey 登录；裁决在论坛的仲裁页面做。 */
export const FORUM_ORIGIN = process.env.NEXT_PUBLIC_FORUM_ORIGIN ?? "https://akari.cc";

/** 后台 Passkey 的 RP ID（ADR 0013 第 4 条）：服务端给的 WebAuthn 参数不是它就拒绝调起。 */
export const ADMIN_RP_ID = process.env.NEXT_PUBLIC_ADMIN_RP_ID ?? "administrator.akari.cc";
