import type { Metadata } from "next";
import { AdminShell } from "../../components/AdminShell";
import { BoardsPage } from "../../components/pages/BoardsPage";

export const metadata: Metadata = { title: "板块" };

export default function Page() {
  return (
    <AdminShell active="/boards/" title="板块">
      <BoardsPage />
    </AdminShell>
  );
}
