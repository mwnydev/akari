import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { ArbitrationPage } from "../../components/pages/ArbitrationPage";

export const metadata: Metadata = { title: "仲裁" };

export default function Page() {
  return (
    <AdminShell active="/arbitration/" title="仲裁总览">
      <ArbitrationPage />
    </AdminShell>
  );
}
