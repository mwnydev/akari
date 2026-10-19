import { LoadingState } from "@akari/ui";
import type { Metadata } from "next";
import { Suspense } from "react";
import { AuctionSignSection } from "../../../components/AuctionSign";
import { WalletNav } from "../../../components/WalletNav";

export const metadata: Metadata = { title: "签署拍卖授权" };

export default function AuctionSignPage() {
  return (
    <>
      <WalletNav />
      <main className="mx-auto max-w-xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">发起拍卖</h1>
        <Suspense fallback={<LoadingState rows={3} />}>
          <AuctionSignSection />
        </Suspense>
      </main>
    </>
  );
}
