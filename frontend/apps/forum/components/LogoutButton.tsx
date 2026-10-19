"use client";

import { createApi, errorMessage } from "@akari/api-client";
import { Button } from "@akari/ui";
import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";

const api = createApi();

export function LogoutButton() {
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
      <Button variant="ghost" size="sm" onClick={logout} disabled={pending}>
        退出
      </Button>
      {error ? (
        <span role="alert" className="text-sm text-danger">
          {error}
        </span>
      ) : null}
    </>
  );
}
