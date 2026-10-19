"use client";

import { errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { MessageCircle } from "lucide-react";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, unwrap } from "../../lib/browser-api";

/** 个人主页上的「发私信」：打开（或新建）与对方的会话再跳过去。 */
export function StartConversation({ userId }: { userId: number }) {
  const router = useRouter();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function open() {
    setPending(true);
    setError(null);
    try {
      const c = await unwrap(api.POST("/api/v1/conversations", { body: { user_id: userId } }));
      router.push(`/messages/${c.id}`);
    } catch (e) {
      setError(errorMessage(e));
      setPending(false);
    }
  }

  return (
    <div className="flex flex-col items-center gap-1 sm:items-end">
      <Button size="sm" onClick={() => void open()} disabled={pending}>
        <MessageCircle size={16} aria-hidden />
        发私信
      </Button>
      {error ? (
        <span role="alert" className="text-sm text-danger">
          {error}
        </span>
      ) : null}
    </div>
  );
}
