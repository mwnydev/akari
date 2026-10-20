import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { AuctionSettingsPage } from "../../components/pages/AuctionSettingsPage";

export const metadata: Metadata = { title: "拍卖设置" };

export default function Page() {
  return (
    <AdminShell active="/auction/" title="拍卖设置">
      <AuctionSettingsPage />
    </AdminShell>
  );
}
