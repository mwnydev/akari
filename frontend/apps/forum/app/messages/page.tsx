import type { Metadata } from "next";
import { AppShell } from "../../components/forum/AppShell";
import { Inbox } from "../../components/messages/Inbox";
import { requireMe } from "../../lib/session";

export const metadata: Metadata = { title: "私信", robots: { index: false } };

export default async function MessagesPage() {
  const me = await requireMe("/messages");
  return (
    <AppShell>
      <div className="max-w-6xl">
        <h1 className="sr-only">私信</h1>
        <Inbox me={me.id} conversationId={null} />
      </div>
    </AppShell>
  );
}
