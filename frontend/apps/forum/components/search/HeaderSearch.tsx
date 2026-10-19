"use client";

import type { Schemas } from "@akari/api-client";
import { Search } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { api } from "../../lib/browser-api";
import { topicHref } from "../../lib/links";
import {
  MAX_QUERY_CHARS,
  SUGGEST_DEBOUNCE_MS,
  SUGGEST_LIMIT,
  searchHref,
  suggestQuery,
} from "../../lib/search";
import { headerIcon } from "../forum/styles";
import { Highlighted } from "./Highlighted";

type TopicResult = Schemas["TopicResult"];

/** 输入停顿 [`SUGGEST_DEBOUNCE_MS`] 后的值。 */
function useDebounced(value: string, ms: number): string {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return debounced;
}

/** 联想：按主题标题搜索的前几条；查询变化时取消上一个请求，失败（含限流）时不显示。 */
function useSuggestions(raw: string): { query: string | null; topics: TopicResult[] } {
  const query = suggestQuery(useDebounced(raw, SUGGEST_DEBOUNCE_MS));
  const [result, setResult] = useState<{ query: string; topics: TopicResult[] } | null>(null);
  useEffect(() => {
    if (!query) return;
    const ctrl = new AbortController();
    api
      .GET("/api/v1/search", {
        params: { query: { q: query, type: "topic" } },
        signal: ctrl.signal,
      })
      .then(({ data }) => {
        if (data) setResult({ query, topics: data.topics.slice(0, SUGGEST_LIMIT) });
      })
      .catch(() => {});
    return () => ctrl.abort();
  }, [query]);
  return { query, topics: query && result?.query === query ? result.topics : [] };
}

/**
 * 顶栏搜索：图标按钮打开一个原生 popover（不需要 JS 也能打开、Esc 关闭），里面是提交到 /search 的普通表单；
 * 有 JS 时输入两个字以上停顿片刻会列出标题匹配的主题。
 */
export function HeaderSearch() {
  const id = useId();
  const panel = `search-${id}`;
  const listId = `search-list-${id}`;
  const [value, setValue] = useState("");
  const { query, topics } = useSuggestions(value);
  return (
    <>
      <button type="button" popoverTarget={panel} aria-label="搜索" className={headerIcon}>
        <Search size={20} aria-hidden />
      </button>
      <div
        id={panel}
        popover="auto"
        aria-label="搜索"
        className="fixed top-14 right-2 bottom-auto left-auto m-0 w-[min(32rem,calc(100vw-1rem))] border border-border bg-surface p-3 text-fg shadow-sheet"
      >
        <form action="/search" role="search" className="flex gap-2">
          <input
            name="q"
            type="search"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            maxLength={MAX_QUERY_CHARS}
            placeholder="搜索帖子、主题"
            aria-label="搜索关键词"
            aria-controls={listId}
            autoComplete="off"
            enterKeyHint="search"
            className="h-10 min-w-0 flex-1 rounded-md border border-border bg-surface px-3 placeholder:text-fg-muted focus-visible:outline-2 focus-visible:outline-accent"
          />
          <button
            type="submit"
            className="h-10 shrink-0 rounded-md bg-accent-fill px-4 text-sm font-medium text-accent-fg hover:opacity-90"
          >
            搜索
          </button>
        </form>
        <ul id={listId} aria-label="匹配的主题" className="mt-2 divide-y divide-border">
          {topics.map((t) => (
            <li key={t.topic.id}>
              <a
                href={topicHref(t.topic.id)}
                className="block px-1 py-2 text-sm hover:bg-surface-2"
              >
                <Highlighted parts={t.title} />
              </a>
            </li>
          ))}
          {query ? (
            <li>
              <a
                href={searchHref({ q: query })}
                className="block px-1 py-2 text-sm text-accent hover:bg-surface-2"
              >
                搜索全部帖子里的「{query}」
              </a>
            </li>
          ) : null}
        </ul>
      </div>
    </>
  );
}
