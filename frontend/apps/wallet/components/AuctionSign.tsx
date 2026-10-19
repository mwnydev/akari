"use client";

import { Amount, Button, Card, CardTitle, ErrorState, LoadingState, shortAddress } from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { useMutation } from "@tanstack/react-query";
import { CircleCheck, PenLine } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useEffect, useState, type ReactNode } from "react";
import { formatCnyPerUsdt } from "../lib/cny";
import { userMessage, walletApi } from "../lib/wallet/api";
import { NETWORK_LABEL, chain } from "../lib/wallet/config";
import { forumReturnUrl } from "../lib/wallet/return-url";
import { loadWalletSdk } from "../lib/wallet/load";
import type { AuctionReview, Wallet } from "../lib/wallet/sdk";
import { AUCTION_ID, AuctionPrice, durationText, windowsText } from "./AuctionTerms";
import { WalletGate } from "./WalletGate";

const FEE_PAYER_TEXT = ["卖家承担", "买家承担", "双方各半"];
/** 签名成功后停留多久再回论坛（读屏软件能播报成功提示）。 */
const RETURN_DELAY_MS = 1500;

function dateTime(secs: bigint): string {
  return new Date(Number(secs) * 1000).toLocaleString("zh-CN", { hour12: false });
}

/** 拍卖授权的每一项条款（就是将要签名的那个对象），加上论坛执行、不进签名的拍卖条件。 */
function authRows({ auth, cny, reserveUsdt, view }: AuctionReview) {
  const signed: { label: string; value: ReactNode }[] = [
    { label: "拍卖编号", value: <span className="amount">#{auth.auctionId.toString()}</span> },
    { label: "计价币种", value: cny ? "人民币（成交后按下方汇率换算 USDT 结算）" : "USDT" },
    ...(cny
      ? [
          {
            label: "你的汇率",
            value: <span className="amount">1 USDT = {formatCnyPerUsdt(auth.cnyPerUsdt)} 元</span>,
          },
        ]
      : []),
    {
      label: "保留价",
      value: (
        <>
          <AuctionPrice cny={cny} amount={auth.reservePrice} />
          {cny ? (
            <span className="block text-xs text-fg-muted">
              折合 <Amount value={reserveUsdt} />
            </span>
          ) : null}
        </>
      ),
    },
    {
      label: "发货期 / 确认期",
      value: windowsText(auth),
    },
    { label: "手续费", value: FEE_PAYER_TEXT[auth.feePayer] ?? "—" },
    {
      label: "最高费率",
      value: <span className="amount">{(auth.maxFeeBps / 100).toFixed(2)}%</span>,
    },
    { label: "最低手续费上限", value: <Amount value={auth.maxMinFee} /> },
    { label: "固定 gas 费上限", value: <Amount value={auth.maxBaseGas} /> },
    { label: "授权有效期至", value: <span className="amount">{dateTime(auth.expiry)}</span> },
    {
      label: "卖家钱包",
      value: <span className="amount font-mono">{shortAddress(auth.seller)}</span>,
    },
    {
      label: "内容快照",
      value: (
        <span className="amount font-mono">
          {auth.listingHash.slice(0, 10)}…{auth.listingHash.slice(-8)}
        </span>
      ),
    },
  ];
  const forum: { label: string; value: ReactNode }[] = [
    { label: "起拍价", value: <AuctionPrice cny={cny} amount={BigInt(view.start_price)} /> },
    { label: "加价幅度", value: <AuctionPrice cny={cny} amount={BigInt(view.increment)} /> },
    { label: "拍卖时长", value: durationText(view.duration_secs) },
  ];
  return { signed, forum };
}

function Rows({ rows, testid }: { rows: { label: string; value: ReactNode }[]; testid?: string }) {
  return (
    <dl className="space-y-2 text-sm" data-testid={testid}>
      {rows.map((r) => (
        <div key={r.label} className="flex justify-between gap-3">
          <dt className="shrink-0 text-fg-muted">{r.label}</dt>
          <dd className="text-right">{r.value}</dd>
        </div>
      ))}
    </dl>
  );
}

function Signed({ back }: { back: string }) {
  useEffect(() => {
    const id = setTimeout(() => location.assign(back), RETURN_DELAY_MS);
    return () => clearTimeout(id);
  }, [back]);
  return (
    <Card className="space-y-3">
      <p
        role="status"
        className="flex items-center gap-2 font-medium text-success"
        data-testid="auction-signed"
      >
        <CircleCheck size={20} aria-hidden />
        拍卖授权已签名并通过链上验证，拍卖已开始
      </p>
      <Button asChild>
        <a href={back}>回到拍卖帖</a>
      </Button>
    </Card>
  );
}

function SignAuction({
  wallet,
  review,
  back,
}: {
  wallet: Wallet;
  review: AuctionReview;
  /** 签名成功后回论坛：上线的拍卖帖 */
  back: (topicId: number) => string;
}) {
  const [open, setOpen] = useState(false);
  const [topic, setTopic] = useState<number | null>(null);
  if (topic !== null) return <Signed back={back(topic)} />;
  const { auth, cny } = review;
  const { signed, forum } = authRows(review);
  return (
    <Card className="space-y-4">
      <CardTitle>签署拍卖授权</CardTitle>
      <section aria-label="签名条款" className="space-y-2">
        <h2 className="text-sm font-medium">签名条款（写进合约，签后不能修改）</h2>
        <Rows rows={signed} testid="auction-terms" />
      </section>
      <section aria-label="拍卖条件" className="space-y-2 border-t border-border pt-3">
        <h2 className="text-sm font-medium">拍卖条件（论坛执行）</h2>
        <Rows rows={forum} />
      </section>
      <p className="text-sm text-fg-muted">
        签名不会动用你的钱：拍卖结束时达到保留价，平台签发成交证明，赢家付款即按这些条款建担保订单；
        平台改不了保留价{cny ? "和汇率" : ""}，你结束时也不用再操作。
      </p>
      <Button className="w-full" onClick={() => setOpen(true)}>
        <PenLine size={18} aria-hidden />
        签署拍卖授权
      </Button>
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action="签署拍卖授权"
        amount="0"
        to={chain.contracts.Escrow}
        toLabel="担保合约"
        network={NETWORK_LABEL}
        rows={[
          { label: "保留价", value: <AuctionPrice cny={cny} amount={auth.reservePrice} /> },
          ...(cny
            ? [{ label: "汇率", value: `1 USDT = ${formatCnyPerUsdt(auth.cnyPerUsdt)} 元` }]
            : []),
        ]}
        note="只是签名，不转出任何资金。发出前可能需要先用 Passkey 确认一次身份。"
        onConfirm={async () => {
          const sdk = await loadWalletSdk();
          const live = await sdk.publishAuctionAuth(walletApi(), wallet, review);
          if (live.topic_id == null) throw new Error("拍卖帖还没有生成，请回到论坛刷新查看");
          setOpen(false);
          setTopic(live.topic_id);
        }}
      />
    </Card>
  );
}

function AuctionSign({
  wallet,
  auctionId,
  back,
}: {
  wallet: Wallet;
  auctionId: number;
  back: (topicId: number) => string;
}) {
  // 授权由后端按当前参数现场生成（需要近期 Passkey 验证），解析与核对在钱包 SDK 里做：
  // 确认页显示的就是将要签名的那个对象
  const review = useMutation({
    mutationFn: async () =>
      (await loadWalletSdk()).startAuctionSigning(walletApi(), wallet, auctionId),
  });
  if (review.isSuccess) return <SignAuction wallet={wallet} review={review.data} back={back} />;
  if (review.isPending) return <LoadingState rows={3} label="正在生成并核对拍卖授权" />;
  return (
    <Card className="space-y-3">
      <CardTitle>签署拍卖授权</CardTitle>
      <p className="text-sm text-fg-muted">
        拍卖 <span className="amount">#{auctionId}</span>
        ：钱包会向论坛取这场拍卖的授权，逐项与链上参数、你在论坛填写的条件核对后再给你确认。
      </p>
      {review.isError ? (
        <ErrorState title="不能签名" message={userMessage(review.error, "拍卖授权校验失败")} />
      ) : null}
      <Button className="w-full" onClick={() => review.mutate()}>
        <PenLine size={18} aria-hidden />
        生成并核对拍卖授权
      </Button>
    </Card>
  );
}

/** `/auction/sign?auction=<拍卖编号>`：卖家挂拍的签名页（从论坛发拍卖帖后跳转过来），签完回到上线的拍卖帖。 */
export function AuctionSignSection() {
  const q = useSearchParams();
  const id = q.get("auction");
  if (!id || !AUCTION_ID.test(id)) {
    return (
      <ErrorState title="链接无效" message="链接不完整，请回到论坛的拍卖帖重新点「去钱包签名」" />
    );
  }
  return (
    <WalletGate>
      {(wallet) => (
        <AuctionSign
          wallet={wallet}
          auctionId={Number(id)}
          back={(topicId) => forumReturnUrl(`/t/${topicId}`)}
        />
      )}
    </WalletGate>
  );
}
