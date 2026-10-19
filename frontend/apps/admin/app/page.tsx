import { AdminShell } from "../components/AdminShell";
import { HomePage } from "../components/pages/HomePage";

export default function Page() {
  return (
    <AdminShell active="/" title="概览">
      <HomePage />
    </AdminShell>
  );
}
