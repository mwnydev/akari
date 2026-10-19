"use client";

import { errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { useMutation } from "@tanstack/react-query";
import { formatRemaining, sessionPhase } from "../lib/session";
import { useNowSecs } from "../lib/useNow";
import { useAdminSession } from "./session/SessionProvider";

/** 会话快到期时的提醒：闲置的可以一键续期，到了最长 12 小时只能重新登录。 */
export function SessionBanner() {
  const { clock, keepAlive } = useAdminSession();
  const now = useNowSecs();
  const extend = useMutation({ mutationFn: keepAlive });
  const p = sessionPhase(clock, now);
  if (p.phase !== "warning") return null;
  return (
    <div
      role="status"
      className="mx-auto mt-3 flex max-w-5xl flex-wrap items-center justify-between gap-2 rounded-md bg-warning-soft px-4 py-2 text-sm text-warning"
    >
      <span>
        {p.reason === "idle"
          ? `后台会话闲置将在 ${formatRemaining(p.remaining)} 后过期`
          : `后台会话已接近最长 12 小时，将在 ${formatRemaining(p.remaining)} 后过期，需重新登录`}
        {extend.isError ? `（续期失败：${errorMessage(extend.error)}）` : ""}
      </span>
      {p.reason === "idle" ? (
        <Button
          size="sm"
          variant="secondary"
          onClick={() => extend.mutate()}
          disabled={extend.isPending}
        >
          继续使用
        </Button>
      ) : null}
    </div>
  );
}
