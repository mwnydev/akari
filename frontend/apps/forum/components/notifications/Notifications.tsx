"use client";

import { QueryProvider } from "../QueryProvider";
import { NotificationList } from "./NotificationList";

export function Notifications() {
  return (
    <QueryProvider>
      <NotificationList />
    </QueryProvider>
  );
}
