import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { AuditPage } from "../../components/pages/AuditPage";

export const metadata: Metadata = { title: "审计日志" };

export default function Page() {
  return (
    <AdminShell active="/audit/" title="审计日志">
      <AuditPage />
    </AdminShell>
  );
}
