import { LoadingState } from "@akari/ui";
import type { Metadata } from "next";
import { Suspense } from "react";
import { AuctionPaySection } from "../../../components/AuctionPay";
import { WalletNav } from "../../../components/WalletNav";

export const metadata: Metadata = { title: "拍卖付款" };

export default function AuctionPayPage() {
  return (
    <>
      <WalletNav />
      <main className="mx-auto max-w-xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">拍卖付款</h1>
        <Suspense fallback={<LoadingState rows={2} />}>
          <AuctionPaySection />
        </Suspense>
      </main>
    </>
  );
}
