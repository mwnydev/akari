"use client";

import { errorMessage } from "@akari/api-client";
import { Card } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import { api, unwrap } from "../../lib/browser-api";
import { formatTime } from "../../lib/display";
import { QueryProvider } from "../QueryProvider";

const RulingPasskeySetup = dynamic(() => import("./RulingPasskeySetup"), { ssr: false });

function Status() {
  const list = useQuery({
    queryKey: ["ruling-passkeys"],
    queryFn: () => unwrap(api.GET("/api/v1/arbitration/passkeys")),
  });
  if (list.isPending) return null;
  if (list.isError) {
    return (
      <p className="text-sm text-fg-muted">裁决 Passkey 状态暂不可用：{errorMessage(list.error)}</p>
    );
  }
  const keys = list.data.passkeys;
  if (!keys.length) return <RulingPasskeySetup onDone={() => void list.refetch()} />;
  const last = keys
    .map((k) => k.last_used_at)
    .filter(Boolean)
    .sort()
    .at(-1);
  return (
    <p className="text-sm text-fg-muted" data-testid="ruling-passkey-status">
      已登记 {keys.length} 个裁决 Passkey{last ? ` · 最近使用 ${formatTime(last)}` : ""}
    </p>
  );
}

/** 仲裁待办页上的裁决 Passkey 状态；还没有时直接引导登记。 */
export function RulingPasskeyCard() {
  return (
    <Card>
      <QueryProvider>
        <Status />
      </QueryProvider>
    </Card>
  );
}
