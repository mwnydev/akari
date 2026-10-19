"use client";

import { createApi, errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { useRouter } from "next/navigation";
import { useState, useTransition, type ReactNode } from "react";

const api = createApi();

/** 账号菜单里的「退出」：注销会话后原地刷新（页面换成游客视图）。 */
export function LogoutButton({ className, children }: { className: string; children: ReactNode }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [error, setError] = useState<string | null>(null);

  async function logout() {
    setError(null);
    try {
      const { error: body, response } = await api.POST("/api/v1/auth/logout");
      if (!response.ok) {
        setError(errorMessage(body));
        return;
      }
    } catch (e) {
      setError(errorMessage(e));
      return;
    }
    startTransition(() => router.refresh());
  }

  return (
    <>
      <Button
        variant="ghost"
        className={className}
        onClick={() => void logout()}
        disabled={pending}
      >
        {children}
      </Button>
      {error ? (
        <p role="alert" className="px-3 py-1 text-xs text-danger">
          {error}
        </p>
      ) : null}
    </>
  );
}
