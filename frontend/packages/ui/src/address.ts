/** 地址显示「前 6 位 … 后 6 位」（plan 第 5、11 节：动钱确认页明文显示）。 */
export function shortAddress(address: string): string {
  if (!/^0x[0-9a-fA-F]{40}$/.test(address)) throw new Error(`invalid address: ${address}`);
  return `${address.slice(0, 8)}…${address.slice(-6)}`;
}
