"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { api, unwrap } from "./api";

export const keys = {
  staff: ["admin-staff"] as const,
  forums: ["admin-forums"] as const,
  suspensions: ["admin-suspensions"] as const,
  settings: ["admin-settings"] as const,
  passkeys: ["admin-passkeys"] as const,
  arbitration: (days: number) => ["admin-arbitration", days] as const,
  execution: ["admin-execution"] as const,
  audit: (actor: number | null, cursor: number | null) => ["admin-audit", actor, cursor] as const,
};

export function useStaff() {
  return useQuery({ queryKey: keys.staff, queryFn: () => unwrap(api.GET("/api/v1/admin/staff")) });
}

export function useForums() {
  return useQuery({
    queryKey: keys.forums,
    queryFn: () => unwrap(api.GET("/api/v1/admin/forums")),
  });
}

/** 任命列表里的用户名（审计日志、工作量只给用户 ID）。 */
export function useStaffNames(): ReadonlyMap<number, string> {
  const { data } = useStaff();
  return useMemo(
    () =>
      new Map(
        (data?.staff ?? []).flatMap((s) => (s.username ? [[s.user_id, s.username] as const] : [])),
      ),
    [data],
  );
}

/** 板块 ID → 名称。 */
export function useForumNames(): ReadonlyMap<number, string> {
  const { data } = useForums();
  return useMemo(() => new Map((data ?? []).map((f) => [f.id, f.name] as const)), [data]);
}
