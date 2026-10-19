import type { Metadata } from "next";
import { SiteNav } from "../../components/SiteNav";
import { Inbox } from "../../components/messages/Inbox";
import { requireMe } from "../../lib/session";

export const metadata: Metadata = { title: "私信", robots: { index: false } };

export default async function MessagesPage() {
  const me = await requireMe("/messages");
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-5xl px-4 py-5">
        <h1 className="sr-only">私信</h1>
        <Inbox me={me.id} conversationId={null} />
      </main>
    </>
  );
}
