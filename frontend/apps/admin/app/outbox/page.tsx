import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { OutboxPage } from "../../components/pages/OutboxPage";

export const metadata: Metadata = { title: "发件箱" };

export default function Page() {
  return (
    <AdminShell active="/outbox/" title="发件箱">
      <OutboxPage />
    </AdminShell>
  );
}
