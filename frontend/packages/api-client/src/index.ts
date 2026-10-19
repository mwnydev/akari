// 类型由 openapi.json 生成（`pnpm gen`），勿手写接口类型：后端 Rust 类型 + utoipa 是唯一定义处。
import createClient, { type ClientOptions } from "openapi-fetch";
import type { components, paths } from "./schema";

export type { components, paths };
export type Schemas = components["schemas"];
export type Me = Schemas["Me"];
export type ErrorCode = Schemas["ErrorCode"];
export type ErrorBody = Schemas["ErrorBody"];

export type ApiClient = ReturnType<typeof createApi>;

/** 浏览器端不传 baseUrl（同源 /api/v1）；服务端渲染传 API 源站地址。 */
export function createApi(options: ClientOptions = {}) {
  return createClient<paths>({ credentials: "same-origin", ...options });
}

/**
 * 错误码的中文兜底提示。正常情况下直接显示后端返回的 `message`；
 * 响应不是 ErrorBody 时用这里。`Record<ErrorCode, …>` 保证后端新增错误码时这里编译不过。
 */
export const ERROR_MESSAGES: Record<ErrorCode, string> = {
  AMOUNT_OVERFLOW: "金额超出范围",
  AMOUNT_UNDERFLOW: "金额超出范围",
  INVALID_PARAMETER: "参数不正确",
  CLAIM_BELOW_GAS: "可领取金额不足以支付领取 gas 费",
  INVALID_EMAIL: "邮箱格式不正确",
  CAPTCHA_FAILED: "人机验证未通过，请重试",
  RESEND_TOO_SOON: "验证码发送太频繁，请稍后再试",
  RATE_LIMITED: "操作太频繁，请稍后再试",
  INVALID_CODE: "验证码错误或已过期",
  TOO_MANY_ATTEMPTS: "错误次数过多，请重新获取验证码",
  UNAUTHORIZED: "请先登录",
  ORIGIN_REJECTED: "请求来源不被允许",
  INTERNAL: "服务暂时不可用，请稍后再试",
};

const NETWORK_MESSAGE = "网络异常，请稍后再试";

export function isErrorBody(v: unknown): v is ErrorBody {
  return (
    typeof v === "object" &&
    v !== null &&
    "code" in v &&
    typeof v.code === "string" &&
    Object.hasOwn(ERROR_MESSAGES, v.code) &&
    "message" in v &&
    typeof v.message === "string"
  );
}

/** 带错误码的异常，供 TanStack Query 的 onError 区分处理。 */
export class ApiError extends Error {
  constructor(
    readonly code: ErrorCode | "NETWORK",
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "ApiError";
  }

  static from(body: unknown, status: number): ApiError {
    if (isErrorBody(body)) {
      return new ApiError(body.code, body.message || ERROR_MESSAGES[body.code], status);
    }
    return new ApiError("NETWORK", NETWORK_MESSAGE, status);
  }
}

/** 把任意错误转成给用户看的中文提示。 */
export function errorMessage(err: unknown): string {
  if (err instanceof ApiError) return err.message;
  if (isErrorBody(err)) return err.message || ERROR_MESSAGES[err.code];
  return NETWORK_MESSAGE;
}
