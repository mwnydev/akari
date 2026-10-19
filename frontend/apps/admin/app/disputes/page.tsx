import type { Metadata } from "next";
import { AdminNav } from "../../components/AdminNav";
import { DisputeBoard } from "../../components/DisputeBoard";

export const metadata: Metadata = { title: "争议面板" };

export default function DisputesPage() {
  return (
    <>
      <AdminNav active="/disputes" />
      <main className="mx-auto max-w-6xl space-y-4 px-4 py-5">
        <h1 className="text-2xl font-semibold">争议面板</h1>
        <DisputeBoard />
      </main>
    </>
  );
}
