import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { SettingsPage } from "../../components/pages/SettingsPage";

export const metadata: Metadata = { title: "限速与阈值" };

export default function Page() {
  return (
    <AdminShell active="/settings/" title="限速与阈值">
      <SettingsPage />
    </AdminShell>
  );
}
