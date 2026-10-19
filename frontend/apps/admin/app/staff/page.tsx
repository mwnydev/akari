import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { StaffPage } from "../../components/pages/StaffPage";

export const metadata: Metadata = { title: "人员" };

export default function Page() {
  return (
    <AdminShell active="/staff/" title="管理人员">
      <StaffPage />
    </AdminShell>
  );
}
