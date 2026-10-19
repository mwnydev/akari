import { BaseError, ContractFunctionRevertedError } from "viem";
import { NO_WITHDRAW_CREDIT } from "./withdraw";

/** 合约自定义错误 → 人话（plan 第 14 节「报错翻译成人话」）。名字与 Escrow / StreamEscrow / EscrowConfig / ERC-20 的 error 一致。 */
const REVERT_MESSAGES: Record<string, string> = {
  BadOffer: "报价条款不符合当前规则（金额上下限、期限或买家不对）",
  OfferUnavailable: "报价已过期、已撤销或已被使用",
  BadSignature: "卖家签名无效",
  FeeLimit: "平台费率已经变化，超出卖家报价时接受的上限，请让卖家重新报价",
  FeesExceedAmount: "手续费超过了订单金额",
  BadState: "订单状态已经变化，请刷新后再试",
  NotParty: "只有订单的买家或卖家可以这样操作",
  TooLate: "已经过了期限",
  TooEarly: "还没到时间（期限未过，或距上次领取不足 24 小时）",
  NothingToClaim: "可领取金额还不够付领取 gas 费",
  NothingToWithdraw: "合约里没有可提取的余额",
  IsPaused: "平台暂停了新订单，请稍后再试",
  TokenNotAllowed: "不支持这个币种",
  TotalLockedCap: "平台托管总额已到上限，请稍后再试",
  ERC20InsufficientBalance: "余额不足",
  ERC20InsufficientAllowance: "授权额度不足",
};

/**
 * forum-sponsor 的业务拒绝（JSON-RPC 错误的 `data.code`）里需要比通用提示更具体说明的。
 * 其余拒绝的 `message` 本身已是后端给的中文提示。
 */
const SPONSOR_MESSAGES: Record<string, string> = {
  WITHDRAW_CREDIT_REQUIRED: NO_WITHDRAW_CREDIT,
};

function sponsorCode(error: BaseError): string | undefined {
  const withCode = error.walk(
    (e) => typeof (e as { data?: { code?: unknown } }).data?.code === "string",
  );
  return (withCode as { data?: { code?: string } } | null)?.data?.code;
}

/** 把 WebAuthn / bundler / 合约报错翻译成用户能看懂的一句话；原始错误放在 cause 里便于排查。 */
export function humanizeError(error: unknown): Error {
  const wrap = (message: string) => new Error(message, { cause: error });
  if (error instanceof Error && error.name === "NotAllowedError") {
    return wrap("Face ID / 指纹验证已取消或超时，没有签名");
  }
  if (error instanceof BaseError) {
    const reverted = error.walk((e) => e instanceof ContractFunctionRevertedError);
    const name =
      reverted instanceof ContractFunctionRevertedError ? reverted.data?.errorName : undefined;
    if (name && REVERT_MESSAGES[name]) return wrap(REVERT_MESSAGES[name]);
    const code = sponsorCode(error);
    if (code && SPONSOR_MESSAGES[code]) return wrap(SPONSOR_MESSAGES[code]);
    const text = `${error.shortMessage} ${error.details}`;
    if (/AA3\d/.test(text)) return wrap("平台代付暂时不可用，请稍后再试");
    if (/AA21/.test(text)) return wrap("钱包里的 ETH 不够付 gas，请先往钱包转一点 ETH");
    if (/AA2\d/.test(text)) return wrap("钱包签名没有通过验证，请重试");
    if (/AA[0-9]{2}/.test(text)) return wrap("交易没有被接受，请稍后再试");
  }
  if (error instanceof Error && /[一-鿿]/.test(error.message)) return error; // 已经是中文提示
  return wrap("链上操作没有完成，请稍后再试");
}
