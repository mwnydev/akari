/** 拼接 className，忽略假值。组件里的条件样式都走这里，不引入额外依赖。 */
export function cn(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ");
}
