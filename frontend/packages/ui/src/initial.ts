/**
 * 头像里的首字：名字的第一个字符（按码点，不拆 emoji 代理对），拉丁字母一律大写（「alice」与「Alice」显示一样），
 * 汉字等没有大小写的原样；空名字显示「?」。
 */
export function avatarInitial(name: string): string {
  const first = Array.from(name.trim())[0];
  return first ? first.toLocaleUpperCase("en-US") : "?";
}
