import { LoadingState } from "@akari/ui";
import type { Metadata } from "next";
import { Suspense } from "react";
import { PaySection } from "../../components/PayOffer";
import { WalletNav } from "../../components/WalletNav";

export const metadata: Metadata = { title: "担保付款" };

export default function PayPage() {
  return (
    <>
      <WalletNav />
      <main className="mx-auto max-w-xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">担保付款</h1>
        <Suspense fallback={<LoadingState rows={2} />}>
          <PaySection />
        </Suspense>
      </main>
    </>
  );
}
