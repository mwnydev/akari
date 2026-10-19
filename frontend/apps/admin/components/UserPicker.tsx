"use client";

import { Field, Input } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useState } from "react";
import { api, unwrap } from "../lib/api";
import { parseUserQuery } from "../lib/users";

export interface PickedUser {
  id: number;
  label: string;
}

/** 选一个用户：输入用户名前缀从候选里点选，或直接输入用户 ID。 */
export function UserPicker({
  label,
  value,
  onChange,
  error,
}: {
  label: string;
  value: PickedUser | null;
  onChange: (u: PickedUser | null) => void;
  error?: string | undefined;
}) {
  const listId = useId();
  const [text, setText] = useState(value?.label ?? "");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setDebounced(text), 250);
    return () => clearTimeout(t);
  }, [text]);
  const q = parseUserQuery(debounced);
  const prefix = q && "prefix" in q ? q.prefix : null;
  const search = useQuery({
    queryKey: ["admin-user-search", prefix],
    queryFn: () =>
      unwrap(api.GET("/api/v1/admin/users/search", { params: { query: { q: prefix ?? "" } } })),
    enabled: prefix !== null && value?.label !== text,
  });

  return (
    <div className="relative">
      <Field label={label} hint="输入用户名搜索，或输入用户 ID（如 #123）" error={error}>
        <Input
          value={text}
          role="combobox"
          aria-expanded={Boolean(search.data?.length)}
          aria-controls={listId}
          onChange={(e) => {
            const next = e.target.value;
            setText(next);
            const parsed = parseUserQuery(next);
            onChange(parsed && "id" in parsed ? { id: parsed.id, label: `#${parsed.id}` } : null);
          }}
        />
      </Field>
      {prefix !== null && value?.label !== text && search.data?.length ? (
        <ul
          id={listId}
          role="listbox"
          className="absolute z-10 mt-1 w-full overflow-hidden rounded-md border border-border bg-surface shadow-sheet"
        >
          {search.data.map((u) => (
            <li key={u.id} role="option" aria-selected={value?.id === u.id}>
              <button
                type="button"
                className="block w-full px-3 py-1.5 text-left text-sm hover:bg-surface-2"
                onClick={() => {
                  const picked = { id: u.id, label: `${u.username}（#${u.id}）` };
                  setText(picked.label);
                  onChange(picked);
                }}
              >
                {u.username} <span className="text-fg-muted">#{u.id}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
