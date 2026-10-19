import { txUrl } from "../lib/wallet/config";

/** 交易哈希：有区块浏览器就给链接，本地链只显示哈希。 */
export function TxLink({ hash }: { hash: string }) {
  const url = txUrl(hash);
  const text = `${hash.slice(0, 10)}…${hash.slice(-8)}`;
  return url ? (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      className="amount font-mono text-accent underline"
    >
      {text}
    </a>
  ) : (
    <span className="amount font-mono" title={hash}>
      {text}
    </span>
  );
}
