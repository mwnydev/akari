import { Amount, Card, CardTitle, NetworkBadge, UserCard } from "@akari/ui";
import { Clock, ShieldCheck, Zap } from "lucide-react";
import type { Metadata } from "next";
import { PayButton } from "../../components/PayButton";
import { WalletNav } from "../../components/WalletNav";
import { checkoutCost, listing, wallet } from "../../lib/mock";

export const metadata: Metadata = { title: "确认下单" };

export default function Checkout() {
  const c = checkoutCost();
  const enough = BigInt(wallet.balance) >= BigInt(c.total);
  return (
    <>
      <WalletNav active="/checkout" />
      <main className="mx-auto max-w-xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">确认下单</h1>
        <Card className="space-y-3">
          <p className="font-medium">{listing.title}</p>
          <UserCard
            name={listing.seller.name}
            level={listing.seller.level}
            online={listing.seller.online}
            reputation={listing.seller.reputation}
            size="sm"
          />
          <ul className="space-y-1.5 text-sm">
            <li className="flex gap-2">
              <Zap size={16} className="mt-1 shrink-0 text-accent" aria-hidden />
              即时发货：付款上链确认后自动展示卡密
            </li>
            <li className="flex gap-2">
              <Clock size={16} className="mt-1 shrink-0 text-accent" aria-hidden />
              确认期 {listing.confirmDays} 天：有问题须在期满前发起争议，否则自动放款给卖家
            </li>
            <li className="flex gap-2">
              <ShieldCheck size={16} className="mt-1 shrink-0 text-accent" aria-hidden />
              <span>
                卖家保证金 <Amount value={listing.bond} /> 将随订单锁定，判卖家责任时可赔付给你
              </span>
            </li>
          </ul>
        </Card>
        <Card>
          <CardTitle className="mb-3">费用明细</CardTitle>
          <dl className="space-y-2">
            {[
              ["货款", c.amount],
              ["你承担的手续费与 gas 费（本单卖家承担）", c.buyerFee],
              ["代付费", c.sponsorFee],
            ].map(([k, v]) => (
              <div key={k} className="flex justify-between gap-3">
                <dt className="text-fg-muted">{k}</dt>
                <dd>
                  <Amount value={v ?? "0"} />
                </dd>
              </div>
            ))}
            <div className="flex justify-between gap-3 border-t border-border pt-2 text-lg font-semibold">
              <dt>合计</dt>
              <dd>
                <Amount value={c.total} />
              </dd>
            </div>
          </dl>
          <p className="mt-3 text-sm text-fg-muted">
            卖家到手 <Amount value={c.sellerGets} />
            。本单争议费 <Amount value={c.disputeFee} />
            ，由发起争议的一方预付，胜诉全额退回。 临近确认期满（最后 6
            小时）钱包没有余额时，可免费紧急顺延 1 次 24 小时，再去交易所提币。
          </p>
        </Card>
        <Card className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="whitespace-nowrap text-fg-muted">钱包余额</span>
            <span className="flex flex-wrap items-center justify-end gap-2">
              <Amount value={wallet.balance} />
              <NetworkBadge />
            </span>
          </div>
          {enough ? (
            <PayButton total={c.total} amount={c.amount} sponsorFee={c.sponsorFee} />
          ) : (
            <p className="rounded-lg bg-warning-soft p-3 text-sm text-warning">
              余额不足，请先从交易所选 Arbitrum One 网络提币到钱包。
            </p>
          )}
        </Card>
      </main>
    </>
  );
}
