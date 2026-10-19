import type { Metadata } from "next";
import { Notifications } from "../../components/notifications/Notifications";
import { AppShell } from "../../components/forum/AppShell";
import { requireMe } from "../../lib/session";

export const metadata: Metadata = { title: "通知", robots: { index: false } };

export default async function NotificationsPage() {
  await requireMe("/notifications");
  return (
    <AppShell>
      <div className="max-w-3xl">
        <Notifications />
      </div>
    </AppShell>
  );
}
