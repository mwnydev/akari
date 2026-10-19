import type { Metadata } from "next";
import { OfferSection } from "../../components/OfferForm";
import { WalletNav } from "../../components/WalletNav";

export const metadata: Metadata = { title: "发起担保" };

export default function OfferPage() {
  return (
    <>
      <WalletNav active="/offer" />
      <main className="mx-auto max-w-xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">发起担保</h1>
        <OfferSection />
      </main>
    </>
  );
}
