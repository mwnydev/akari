import { Amount, Button, Card, CardTitle, NetworkBadge, shortAddress } from "@akari/ui";
import { Copy, TriangleAlert } from "lucide-react";
import { WalletNav } from "../components/WalletNav";
import { Withdraw } from "../components/Withdraw";
import { wallet } from "../lib/mock";

/** 设计稿用的占位二维码图案（不是真实编码），接入后换成真实二维码。 */
function QrPlaceholder() {
  const cells = Array.from({ length: 21 * 21 }, (_, i) => ((i * 7919) % 13) % 3 === 0);
  return (
    <svg
      viewBox="0 0 21 21"
      className="size-36 rounded-md bg-white p-2"
      role="img"
      aria-label="充值地址二维码（示意）"
    >
      {cells.map((on, i) =>
        on ? (
          <rect key={i} x={i % 21} y={Math.floor(i / 21)} width="1" height="1" fill="#000" />
        ) : null,
      )}
      {[0, 14].flatMap((x) =>
        [0, 14]
          .filter((y) => !(x === 14 && y === 14))
          .map((y) => (
            <g key={`${x}-${y}`}>
              <rect x={x} y={y} width="7" height="7" fill="#000" />
              <rect x={x + 1} y={y + 1} width="5" height="5" fill="#fff" />
              <rect x={x + 2} y={y + 2} width="3" height="3" fill="#000" />
            </g>
          )),
      )}
    </svg>
  );
}

export default function WalletHome() {
  return (
    <>
      <WalletNav active="/" />
      <main className="mx-auto grid max-w-5xl gap-4 px-4 py-5 lg:grid-cols-2">
        <Card className="lg:col-span-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-sm text-fg-muted">可用余额</p>
            <NetworkBadge />
          </div>
          <p className="mt-1 text-4xl font-semibold">
            <Amount value={wallet.balance} />
          </p>
          <p className="mt-1 text-sm text-fg-muted">
            钱包不需要 ETH，gas 由平台代付。建议交易完成后提回交易所，不在钱包长期存放。
          </p>
        </Card>
        <Card>
          <CardTitle className="mb-3">充值</CardTitle>
          <div className="flex flex-col items-center gap-3 sm:flex-row sm:items-start">
            <QrPlaceholder />
            <div className="min-w-0 flex-1 space-y-2">
              <p className="text-sm text-fg-muted">你的钱包地址（Arbitrum One）</p>
              <p className="amount font-mono text-sm break-all">{wallet.address}</p>
              <Button variant="secondary" size="sm">
                <Copy size={16} aria-hidden />
                复制地址（{shortAddress(wallet.address)}）
              </Button>
            </div>
          </div>
          <div
            role="note"
            className="mt-4 flex gap-2 rounded-lg bg-warning-soft p-3 text-sm text-warning"
          >
            <TriangleAlert size={18} className="mt-0.5 shrink-0" aria-hidden />
            <p>
              交易所提币时网络必须选 <strong>Arbitrum One</strong>。选 TRC20（波场）的币
              <strong>无法找回</strong>；选错其他 EVM 网络可以提交工单找回。
            </p>
          </div>
        </Card>
        <Card>
          <CardTitle className="mb-3">提回交易所</CardTitle>
          <Withdraw />
        </Card>
        <Card className="p-0 sm:p-0 lg:col-span-2">
          <CardTitle className="px-4 pt-4 sm:px-5">交易记录</CardTitle>
          <ul className="divide-y divide-border">
            {wallet.history.map((h) => (
              <li key={h.id} className="flex items-center justify-between gap-3 px-4 py-3 sm:px-5">
                <div>
                  <p className="font-medium">{h.kind}</p>
                  <p className="amount text-sm text-fg-muted">
                    {h.at} · {h.tx}
                  </p>
                </div>
                <p
                  className={h.amount.startsWith("-") ? "font-medium" : "font-medium text-success"}
                >
                  {h.amount.startsWith("-") ? "" : "+"}
                  <Amount value={h.amount} />
                </p>
              </li>
            ))}
          </ul>
        </Card>
      </main>
    </>
  );
}
