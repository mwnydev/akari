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
  CAPTCHA_REQUIRED: "注册 24 小时内发帖需要先完成人机验证",
  RESEND_TOO_SOON: "验证码发送太频繁，请稍后再试",
  RATE_LIMITED: "操作太频繁，请稍后再试",
  INVALID_CODE: "验证码错误或已过期",
  TOO_MANY_ATTEMPTS: "错误次数过多，请重新获取验证码",
  UNAUTHORIZED: "请先登录",
  ORIGIN_REJECTED: "请求来源不被允许",
  NOT_FOUND: "内容不存在或已被删除",
  FORBIDDEN: "没有权限进行此操作",
  TOPIC_LOCKED: "主题已锁定，不能回复或编辑",
  CONTENT_EMPTY: "内容不能为空",
  CONTENT_TOO_LONG: "内容太长",
  INVALID_TITLE: "标题需为 1–120 个字",
  EXTERNAL_LINK_FORBIDDEN: "注册 24 小时内不能发站外链接",
  AMOUNT_OUT_OF_RANGE: "订单金额超出允许范围",
  WINDOW_OUT_OF_RANGE: "发货期限和确认期需在 24 小时到 30 天之间",
  BOND_INVALID: "保证金不能超过订单金额，违约比例不能超过 100%",
  FEE_LIMIT_INVALID: "最高费率或最高 gas 费不能低于当前收费，也不能超过合约上限",
  OFFER_EXPIRY_INVALID: "报价有效期不正确",
  OFFER_UNAVAILABLE: "报价已失效",
  CONFLICT: "已经存在，不能重复创建",
  USERNAME_REQUIRED: "请先设置用户名",
  INVALID_USERNAME: "用户名需为 2–20 个字，只能包含汉字、字母、数字和下划线，不能是纯数字或保留名",
  USERNAME_TAKEN: "用户名已被使用",
  USERNAME_CHANGE_TOO_SOON: "用户名 30 天内只能修改一次",
  TRADE_NOT_ALLOWED: "这个板块不能发交易帖",
  AUCTION_NOT_ALLOWED: "这个板块不能发拍卖帖，请到拍卖区发帖",
  FIRST_POST_NO_COMMENTS: "主楼不能评论，请直接回复主题",
  UNSUPPORTED_IMAGE: "只支持 JPG、PNG、GIF、WebP、HEIC 图片",
  IMAGE_TOO_LARGE: "图片不能超过 10 MB",
  UPLOADS_UNAVAILABLE: "图片上传暂时不可用，请稍后再试",
  REVERIFY_REQUIRED: "请先完成邮箱验证",
  PASSKEY_REQUIRED: "请先用钱包 Passkey 确认",
  PASSKEY_FAILED: "Passkey 验证失败，请重试",
  TOO_MANY_PASSKEYS: "Passkey 数量已达上限",
  WALLET_REQUIRED: "请先开通交易",
  WALLET_ADDRESS_MISMATCH: "钱包地址与 Passkey 不匹配，请刷新后重试",
  SPONSOR_NOT_ALLOWED: "这项操作不在平台代付范围内",
  SPONSOR_QUOTA_EXCEEDED: "这项操作的代付次数已用完，请稍后再试",
  SPONSOR_PENDING: "上一笔操作还在上链，请稍候再试",
  SPONSOR_SUSPENDED: "链上失败次数过多，代付已暂停 24 小时",
  SIMULATION_FAILED: "交易预演失败，请检查余额和订单状态",
  SPONSOR_UNAVAILABLE: "代付服务暂时不可用，请稍后再试",
  WITHDRAW_ADDRESS_INVALID: "地址格式不正确，请检查后重新粘贴",
  WITHDRAW_ADDRESS_FORBIDDEN: "不能使用本人钱包地址或平台合约地址",
  WITHDRAW_ADDRESS_LIMIT: "最多保存 10 个提币地址",
  WITHDRAW_ADDRESS_NOT_ACTIVE: "只能提到已生效的提币地址",
  WITHDRAW_CREDIT_REQUIRED:
    "免费代付提币次数已用完：完成一笔担保交易后可再获得 1 次；也可在「紧急取回」页自付 ETH 提取",
  CHAIN_UNAVAILABLE: "暂时无法读取链上数据，请稍后再试",
  USER_BLOCKED: "你已屏蔽对方或被对方屏蔽，无法发送私信",
  STAFF_SUSPENDED: "你的管理权限已被暂停，请联系站长",
  MUTED: "你已被禁言",
  ACCOUNT_BANNED: "账号已被封禁",
  AUCTION_BANNED: "你已被禁止参与拍卖",
  ACCESS_BANNED: "当前网络或设备已被禁止登录",
  SANCTION_TOO_LONG: "处罚时长超出你的权限范围",
  ADMIN_REVERIFY_REQUIRED: "请先用后台 Passkey 再次验证",
  CASE_STATE_INVALID: "案件当前阶段不能进行这项操作",
  EVIDENCE_WINDOW_OPEN: "举证期还没结束，结束后才能裁决",
  EVIDENCE_WINDOW_CLOSED: "举证期已结束，不能再提交陈述或证据",
  CHAT_CHAIN_BROKEN: "订单聊天记录校验失败（可能被改动），暂停裁决并已记录，请联系站长",
  REPORT_RESOLVED: "这条举报已经处理过了",
  RULING_MISMATCH: "提交的裁决与 Passkey 确认的内容不一致，请重新确认",
  AUCTIONS_DISABLED: "拍卖功能暂未开放",
  AUCTION_UNAVAILABLE: "拍卖当前状态不能进行这项操作",
  AUCTION_DURATION_INVALID: "拍卖时长超出允许的范围",
  AUCTION_INCREMENT_TOO_LOW: "加价幅度不能低于 1 USDT，也不能低于起拍价的 1%",
  AUCTION_SIGN_EXPIRED: "签名已超时，请重新发起签名",
  AUCTION_SIGNATURE_INVALID: "拍卖授权签名无效，请重新签名",
  AUCTION_HAS_BIDS: "已经有人出价，不能取消拍卖",
  BID_TOO_LOW: "出价低于当前最低出价，请刷新后重试",
  BID_BY_SELLER: "卖家本人及关联账号不能出价",
  BID_ALREADY_LEADING: "你已是最高出价者",
  BID_NEW_ACCOUNT_CAP: "新注册账号单场出价有上限（默认注册满 7 天后解除）",
  BID_BALANCE_INSUFFICIENT: "钱包 USDT 余额未达到卖家要求的出价资格",
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

/**
 * webauthn-rs 的 `CreationChallengeResponse` / `RequestChallengeResponse` 外面包着一层 `publicKey`，里面才是
 * @simplewebauthn/browser 的 `optionsJSON`。取出时核对 RP ID：后台、裁决、钱包三组 Passkey 各有自己的 RP ID
 * （ADR 0012、0013），服务端配错或接口被混用时在调起系统弹窗之前就停下。
 */
export function passkeyOptions<T extends { rp?: { id?: string } } | { rpId?: string }>(
  options: unknown,
  rpId: string,
): T {
  const inner = (options as { publicKey?: T } | null)?.publicKey;
  const got = !inner
    ? undefined
    : "rp" in inner
      ? inner.rp?.id
      : "rpId" in inner
        ? inner.rpId
        : undefined;
  if (!inner || got !== rpId) {
    throw new ApiError("PASSKEY_FAILED", "Passkey 参数与本站不符，请刷新后重试", 0);
  }
  return inner;
}

/** 调起系统 Passkey 弹窗：被取消、超时等浏览器错误换成带中文提示的 ApiError，界面统一用 errorMessage 显示。 */
export async function passkeyCeremony<T>(run: () => Promise<T>): Promise<T> {
  try {
    return await run();
  } catch (e) {
    if (e instanceof ApiError) throw e;
    throw new ApiError("PASSKEY_FAILED", "Passkey 验证没有完成（已取消或超时），请重试", 0);
  }
}
