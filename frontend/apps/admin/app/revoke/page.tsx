import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { RevokePage } from "../../components/pages/RevokePage";

export const metadata: Metadata = { title: "批量撤销" };

export default function Page() {
  return (
    <AdminShell active="/revoke/" title="批量撤销">
      <RevokePage />
    </AdminShell>
  );
}
