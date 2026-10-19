import { WalletHome } from "../components/WalletHome";
import { WalletNav } from "../components/WalletNav";

export default function WalletHomePage() {
  return (
    <>
      <WalletNav active="/" />
      <main className="mx-auto max-w-5xl px-4 py-5">
        <WalletHome />
      </main>
    </>
  );
}
