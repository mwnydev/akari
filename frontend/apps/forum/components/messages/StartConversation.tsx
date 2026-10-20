"use client";

import { errorMessage } from "@akari/api-client";
import { Button, type ButtonVariant } from "@akari/ui";
import { MessageCircle, ShieldCheck } from "lucide-react";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, unwrap } from "../../lib/browser-api";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { usePreview } from "../forum/preview";
import { useUsernameGate } from "../settings/UsernameGate";

/**
 * 打开（或新建）与对方的私信会话，然后：`chat` 跳到会话；`offer` 去钱包在这个会话里发起担保
 * （与私信页的「发起担保」同一个入口：`/offer?conversation=…`）。
 */
export function StartConversation({
  userId,
  then = "chat",
  label,
  variant = "primary",
  size = "sm",
  className,
}: {
  userId: number;
  then?: "chat" | "offer";
  label?: string;
  variant?: ButtonVariant;
  size?: "sm" | "md";
  className?: string;
}) {
  const router = useRouter();
  const preview = usePreview();
  const gate = useUsernameGate();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function open() {
    if (preview) return;
    setPending(true);
    setError(null);
    try {
      const c = await unwrap(api.POST("/api/v1/conversations", { body: { user_id: userId } }));
      if (then === "offer") location.assign(`${WALLET_ORIGIN}/offer?conversation=${c.id}`);
      else router.push(`/messages/${c.id}`);
    } catch (e) {
      setError(errorMessage(e));
      setPending(false);
    }
  }

  const Icon = then === "offer" ? ShieldCheck : MessageCircle;
  return (
    <div className="flex flex-col gap-1">
      <Button
        size={size}
        variant={variant}
        className={className}
        // 发起担保要先有用户名（钱包开通交易要求），没有就先弹出设置
        onClick={() => (then === "offer" ? gate.ask(() => void open()) : void open())}
        disabled={pending}
      >
        <Icon size={16} aria-hidden />
        {label ?? (then === "offer" ? "发起担保" : "发私信")}
      </Button>
      {error ? (
        <span role="alert" className="text-sm text-danger">
          {error}
        </span>
      ) : null}
    </div>
  );
}
