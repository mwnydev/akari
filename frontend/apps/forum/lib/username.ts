// 设置用户名前的本地检查（只为即时提示；保留名、是否已被占用以后端为准）。
// 规则同后端 `forum_core::username`：2–20 个字，汉字、ASCII 字母、数字、下划线，不能是纯数字。
import { ERROR_MESSAGES } from "@akari/api-client";

export const USERNAME_MAX_CHARS = 20;
/** 用户名允许的字符（同后端 `is_name_char`）：ASCII 字母数字、下划线、常用汉字（基本区与扩展 A 区）。 */
export const USERNAME_CHARS = /^[A-Za-z0-9_\u3400-\u4DBF\u4E00-\u9FFF]+$/u;

/** 不合规时返回提示文字，合规返回 null。 */
export function usernameProblem(raw: string): string | null {
  const name = raw.trim();
  const n = Array.from(name).length;
  if (n === 0) return "请输入用户名";
  if (n < 2 || n > USERNAME_MAX_CHARS || !USERNAME_CHARS.test(name) || /^\d+$/.test(name)) {
    return ERROR_MESSAGES.INVALID_USERNAME;
  }
  return null;
}
