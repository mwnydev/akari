import type { Metadata } from "next";
import { WalletNav } from "../../components/WalletNav";
import { WalletSetup } from "../../components/WalletSetup";

export const metadata: Metadata = { title: "开通交易" };

export default function SetupPage() {
  return (
    <>
      <WalletNav active="/setup" />
      <main className="mx-auto max-w-xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">开通交易</h1>
        <WalletSetup />
      </main>
    </>
  );
}
