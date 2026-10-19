import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { SuspensionsPage } from "../../components/pages/SuspensionsPage";

export const metadata: Metadata = { title: "权限暂停" };

export default function Page() {
  return (
    <AdminShell active="/suspensions/" title="权限暂停">
      <SuspensionsPage />
    </AdminShell>
  );
}
