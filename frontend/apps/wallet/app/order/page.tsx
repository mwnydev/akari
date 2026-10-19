import { LoadingState } from "@akari/ui";
import type { Metadata } from "next";
import { Suspense } from "react";
import { OrderSection } from "../../components/OrderLive";
import { WalletNav } from "../../components/WalletNav";

export const metadata: Metadata = { title: "订单" };

export default function OrderPage() {
  return (
    <>
      <WalletNav active="/order" />
      <main className="mx-auto max-w-5xl px-4 py-5">
        <Suspense fallback={<LoadingState rows={2} />}>
          <OrderSection />
        </Suspense>
      </main>
    </>
  );
}
