import { ApiError, type Schemas } from "@akari/api-client";
import { Avatar, Button, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import type { ReactNode } from "react";
import { headers } from "next/headers";
import Link from "next/link";
import { AppShell } from "../../components/forum/AppShell";
import { BoardMark } from "../../components/forum/BoardMark";
import { selectClass } from "../../components/forum/styles";
import { PageError } from "../../components/PageError";
import { Highlighted } from "../../components/search/Highlighted";
import { load, serverApi } from "../../lib/api";
import { boardColor } from "../../lib/board-color";
import { cardLookup, formatTime, profileHref, userName } from "../../lib/display";
import { listForums } from "../../lib/forums";
import { boardHref, topicHref } from "../../lib/links";
import { MAX_QUERY_CHARS, parseSearchParams, SEARCH_TYPES, searchHref } from "../../lib/search";

type Search = Promise<Record<string, string | string[] | undefined>>;

// 搜索结果页不进搜索引擎索引（关键词任意，容易被拿来刷垃圾页面）
export const metadata: Metadata = { title: "搜索", robots: { index: false, follow: true } };

/**
 * 服务端渲染时把访问者的 IP 带给后端：搜索按 IP 限流，不带的话所有游客共用 Next.js 所在机器的一个名额。
 * 只转发 Cloudflare 设置的 `CF-Connecting-IP`（后端也只在连接来自本机时才信任它）。
 */
async function clientIpHeader(): Promise<Record<string, string>> {
  const ip = (await headers()).get("cf-connecting-ip");
  return ip ? { "cf-connecting-ip": ip } : {};
}

/**
 * 搜索页：默认「全部」（主题标题、帖子正文，关键词像用户名时还有用户，各带命中数），也可以只看帖子、主题、用户；
 * `?board=` 只搜一个板块，`?cursor=` 翻到更早的结果（单一类型才翻页）。
 */
export default async function SearchPage({ searchParams }: { searchParams: Search }) {
  const p = parseSearchParams(await searchParams);
  const forums = await listForums();
  const result = p.q
    ? await load(
        (await serverApi()).GET("/api/v1/search", {
          params: {
            query: { q: p.q, type: p.type, board: p.board ?? null, cursor: p.cursor ?? null },
          },
          headers: await clientIpHeader(),
        }),
      )
    : null;
  const boards = forums instanceof ApiError ? [] : forums;

  return (
    <AppShell>
      <h1 className="mb-4 text-2xl leading-snug font-semibold">搜索</h1>
      <form action="/search" role="search" className="mb-4 flex flex-wrap gap-2">
        <input
          name="q"
          type="search"
          defaultValue={p.q}
          maxLength={MAX_QUERY_CHARS}
          placeholder="关键词，空格分隔的多个词须同时出现"
          aria-label="搜索关键词"
          className="h-10 min-w-0 flex-[1_1_16rem] rounded-md border border-border bg-surface px-3 placeholder:text-fg-muted focus-visible:outline-2 focus-visible:outline-accent"
        />
        <select
          name="board"
          defaultValue={p.board ?? ""}
          aria-label="板块"
          className={`${selectClass} w-auto flex-none`}
        >
          <option value="">全部板块</option>
          {boards.map((f) => (
            <option key={f.id} value={f.slug}>
              {f.name}
            </option>
          ))}
        </select>
        {p.type !== "all" ? <input type="hidden" name="type" value={p.type} /> : null}
        <Button type="submit">搜索</Button>
      </form>
      <nav aria-label="搜索范围" className="mb-2 flex gap-1 border-b border-border">
        {SEARCH_TYPES.map((t) => (
          <Link
            key={t.value}
            href={p.q ? searchHref({ ...p, type: t.value, cursor: undefined }) : "/search"}
            aria-current={t.value === p.type ? "page" : undefined}
            className={
              t.value === p.type
                ? "-mb-px border-b-2 border-accent px-3 py-2 text-sm font-medium"
                : "px-3 py-2 text-sm text-fg-muted hover:text-fg"
            }
          >
            {t.label}
          </Link>
        ))}
      </nav>
      {result === null ? (
        <EmptyState title="输入关键词开始搜索" description="中文、英文、数字都可以，不区分大小写" />
      ) : result instanceof ApiError ? (
        result.status === 404 ? (
          <EmptyState title="没有这个板块" />
        ) : (
          <PageError error={result} title="搜索失败" />
        )
      ) : (
        <Results data={result} q={p} />
      )}
    </AppShell>
  );
}

type Data = Schemas["SearchResponse"];
type Params = ReturnType<typeof parseSearchParams>;

function lookups(data: Data) {
  const user = cardLookup(data.users);
  const forum = new Map(data.forums.map((f) => [f.id, f]));
  const board = (id: number) => {
    const f = forum.get(id);
    return f ? { name: f.name, color: boardColor(f), href: boardHref(f.slug) } : null;
  };
  return { user, board };
}

function PostItems({ data }: { data: Data }) {
  const { user, board } = lookups(data);
  return data.posts.map((r) => {
    const b = board(r.forum_id);
    const author = user(r.author_id);
    return (
      <li key={`p${r.id}`} className="py-3">
        <Link href={topicHref(r.topic_id, r.page, r.floor)} className="font-medium hover:underline">
          {r.topic_title}
        </Link>
        <p className="mt-1 text-sm leading-relaxed break-words">
          <Highlighted parts={r.snippet} before={r.truncated_start} after={r.truncated_end} />
        </p>
        <p className="mt-1 flex flex-wrap items-center gap-x-3 text-xs text-fg-muted">
          {b ? <BoardMark board={b} link /> : null}
          <Link href={profileHref(author)} className="hover:text-fg">
            {userName(author)}
          </Link>
          <span>#{r.floor}</span>
          <time dateTime={r.created_at}>{formatTime(r.created_at)}</time>
        </p>
      </li>
    );
  });
}

function TopicItems({ data }: { data: Data }) {
  const { user, board } = lookups(data);
  return data.topics.map(({ topic: t, title }) => {
    const b = board(t.forum_id);
    return (
      <li key={`t${t.id}`} className="py-3">
        <Link href={topicHref(t.id)} className="font-medium hover:underline">
          <Highlighted parts={title} />
        </Link>
        <p className="mt-1 flex flex-wrap items-center gap-x-3 text-xs text-fg-muted">
          {b ? <BoardMark board={b} link /> : null}
          <span>{userName(user(t.author_id))}</span>
          <span>{t.replies} 回复</span>
          <time dateTime={t.last_post_at}>{formatTime(t.last_post_at)}</time>
        </p>
      </li>
    );
  });
}

function PeopleItems({ data }: { data: Data }) {
  return data.people.map((u) => (
    <li key={`u${u.id}`} className="py-2">
      <Link href={profileHref(u)} className="flex items-center gap-3 hover:underline">
        <Avatar name={u.username} src={u.avatar_url ?? undefined} size={32} />
        <span>{u.username}</span>
        <span className="text-xs text-fg-muted">{u.level} 级</span>
      </Link>
    </li>
  ));
}

const notFound = <EmptyState title="没有找到相关内容" description="换个词试试，或者减少关键词" />;

/** 「全部」的一组结果：标题带命中数，「查看全部」切到这一类（可以翻页）。 */
function Group({
  title,
  count,
  more,
  children,
}: {
  title: string;
  count: string;
  more: string | null;
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="mb-4">
      <h2 className="flex items-baseline gap-2 border-b border-border py-2 text-sm font-medium">
        {title}
        <span className="text-xs font-normal text-fg-muted">{count}</span>
        {more ? (
          <Link href={more} className="ml-auto text-xs font-normal text-accent hover:underline">
            查看全部
          </Link>
        ) : null}
      </h2>
      <ul className="divide-y divide-border">{children}</ul>
    </section>
  );
}

function AllResults({ data, q }: { data: Data; q: Params }) {
  if (data.posts.length === 0 && data.topics.length === 0 && data.people.length === 0) {
    return notFound;
  }
  const about = (n: number) => `约 ${n.toLocaleString("zh-CN")} 条`;
  const tab = (type: "post" | "topic" | "user") => searchHref({ ...q, type, cursor: undefined });
  return (
    <>
      {data.topics.length > 0 ? (
        <Group
          title="主题"
          count={about(data.topic_total)}
          more={data.topic_total > data.topics.length ? tab("topic") : null}
        >
          <TopicItems data={data} />
        </Group>
      ) : null}
      {data.posts.length > 0 ? (
        <Group
          title="帖子"
          count={about(data.total)}
          more={data.total > data.posts.length ? tab("post") : null}
        >
          <PostItems data={data} />
        </Group>
      ) : null}
      {data.people.length > 0 ? (
        <Group title="用户" count={`${data.people.length} 人`} more={tab("user")}>
          <PeopleItems data={data} />
        </Group>
      ) : null}
    </>
  );
}

function Results({ data, q }: { data: Data; q: Params }) {
  if (q.type === "all") return <AllResults data={data} q={q} />;
  const empty = data.posts.length === 0 && data.topics.length === 0 && data.people.length === 0;
  if (empty) {
    return q.cursor ? <EmptyState title="没有更多结果了" /> : notFound;
  }
  return (
    <>
      {q.type !== "user" && !q.cursor ? (
        <p className="py-2 text-sm text-fg-muted">约 {data.total.toLocaleString("zh-CN")} 条结果</p>
      ) : null}
      <ul className="divide-y divide-border">
        <PostItems data={data} />
        <TopicItems data={data} />
        <PeopleItems data={data} />
      </ul>
      {data.next_cursor ? (
        <p className="py-3 text-center">
          <Button asChild size="sm" variant="secondary">
            <Link href={searchHref({ ...q, cursor: data.next_cursor })}>更早的结果</Link>
          </Button>
        </p>
      ) : null}
    </>
  );
}
