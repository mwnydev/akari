"use client";

import { Button, Card, CardTitle, NetworkBadge, shortAddress } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { Check, Copy, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { NETWORK_LABEL } from "../lib/wallet/config";

/** 地址二维码：纯地址文本（交易所 App 扫码只认地址），编码器按需加载，不进首屏 JS。 */
function AddressQr({ address }: { address: string }) {
  const [path, setPath] = useState<{ d: string; size: number } | null>(null);
  useEffect(() => {
    let live = true;
    import("uqr").then(({ encode }) => {
      const { data, size } = encode(address, { ecc: "M", border: 2 });
      const d = data
        .flatMap((row, y) => row.map((on, x) => (on ? `M${x} ${y}h1v1h-1z` : "")))
        .join("");
      if (live) setPath({ d, size });
    });
    return () => {
      live = false;
    };
  }, [address]);
  return (
    <svg
      viewBox={path ? `0 0 ${path.size} ${path.size}` : "0 0 1 1"}
      className="size-40 shrink-0 rounded-md bg-white"
      shapeRendering="crispEdges"
      role="img"
      aria-label={`充值地址二维码：${address}`}
      data-testid="deposit-qr"
    >
      {path ? <path d={path.d} fill="#000" /> : null}
    </svg>
  );
}

/** 充值卡片：地址、二维码、网络标识，醒目提示网络必须选构建所在的链（plan 第 5 节「充值」；测试网显示 Arbitrum Sepolia）。 */
export function DepositCard({ address }: { address: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("复制失败，请长按地址手动复制");
    }
  }
  return (
    <Card>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <CardTitle>充值</CardTitle>
        <NetworkBadge label={NETWORK_LABEL} />
      </div>
      <div
        role="alert"
        className="mb-4 flex gap-2 rounded-lg bg-danger-soft p-3 text-sm font-medium text-danger"
      >
        <TriangleAlert size={18} className="mt-0.5 shrink-0" aria-hidden />
        <p>
          交易所提币请选 <strong>{NETWORK_LABEL}</strong> 网络，选 <strong>TRC20</strong> 无法找回。
        </p>
      </div>
      <div className="flex flex-col items-center gap-3 sm:flex-row sm:items-start">
        <AddressQr address={address} />
        <div className="min-w-0 flex-1 space-y-2">
          <p className="text-sm text-fg-muted">你的钱包地址（只收 {NETWORK_LABEL} 上的 USDT）</p>
          <p className="amount font-mono text-sm break-all" data-testid="deposit-address">
            {address}
          </p>
          <Button variant="secondary" size="sm" onClick={copy}>
            {copied ? <Check size={16} aria-hidden /> : <Copy size={16} aria-hidden />}
            {copied ? "已复制" : `复制地址（${shortAddress(address)}）`}
          </Button>
        </div>
      </div>
      <p className="mt-3 text-sm text-fg-muted">
        选错其他 EVM 网络（如 BSC、以太坊）的币不会自动到账，可以提交工单找回。
      </p>
    </Card>
  );
}
