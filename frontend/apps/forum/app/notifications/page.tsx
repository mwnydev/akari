import type { Metadata } from "next";
import { Notifications } from "../../components/notifications/Notifications";
import { SiteNav } from "../../components/SiteNav";
import { requireMe } from "../../lib/session";

export const metadata: Metadata = { title: "通知", robots: { index: false } };

export default async function NotificationsPage() {
  await requireMe("/notifications");
  return (
    <>
      <SiteNav />
      <main className="mx-auto max-w-3xl px-4 py-5">
        <Notifications />
      </main>
    </>
  );
}
