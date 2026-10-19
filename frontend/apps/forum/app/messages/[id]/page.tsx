import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { AppShell } from "../../../components/forum/AppShell";
import { Inbox } from "../../../components/messages/Inbox";
import { requireMe } from "../../../lib/session";

export const metadata: Metadata = { title: "私信", robots: { index: false } };

export default async function ConversationPage({ params }: { params: Promise<{ id: string }> }) {
  const raw = (await params).id;
  if (!/^\d{1,15}$/.test(raw)) notFound();
  const me = await requireMe(`/messages/${raw}`);
  return (
    <AppShell>
      <div className="max-w-6xl">
        <h1 className="sr-only">私信</h1>
        <Inbox me={me.id} conversationId={Number(raw)} />
      </div>
    </AppShell>
  );
}
