import { ApiError } from "@akari/api-client";
import { Button, Card, Input } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { cache } from "react";
import { ComposerLauncher } from "../../../components/composer/ComposerLauncher";
import { PageError } from "../../../components/PageError";
import { Pager } from "../../../components/Pager";
import { SiteNav } from "../../../components/SiteNav";
import { TopicBadges } from "../../../components/TopicBadges";
import { Floor } from "../../../components/topic/Floor";
import { PostModMenu } from "../../../components/moderation/PostModMenu";
import { TopicModMenu } from "../../../components/moderation/TopicModMenu";
import { load, serverApi } from "../../../lib/api";
import { cardLookup, userName } from "../../../lib/display";
import { listForums } from "../../../lib/forums";
import { boardHref, loginHref, topicHref } from "../../../lib/links";
import { floorPage, parsePositiveInt } from "../../../lib/pagination";
import { getMe } from "../../../lib/session";

type Params = Promise<{ id: string }>;
type Search = Promise<Record<string, string | string[] | undefined>>;

const loadTopic = cache(async (id: number, page: number) =>
  load(
    (await serverApi()).GET("/api/v1/topics/{id}", {
      params: { path: { id }, query: { page } },
    }),
  ),
);

function topicId(raw: string): number {
  if (!/^\d{1,15}$/.test(raw)) notFound();
  return Number(raw);
}

export async function generateMetadata({
  params,
  searchParams,
}: {
  params: Params;
  searchParams: Search;
}): Promise<Metadata> {
  const id = topicId((await params).id);
  const data = await loadTopic(id, parsePositiveInt((await searchParams).page));
  return data instanceof ApiError ? {} : { title: data.topic.title };
}

export default async function TopicPage({
  params,
  searchParams,
}: {
  params: Params;
  searchParams: Search;
}) {
  const id = topicId((await params).id);
  const sp = await searchParams;
  // 按楼层跳页（plan 第 4 节）：普通 GET 表单提交 ?floor=N，服务端换算页码后跳到该楼
  if (sp.floor !== undefined) {
    const floor = parsePositiveInt(sp.floor);
    redirect(topicHref(id, floorPage(floor), floor));
  }
  const page = parsePositiveInt(sp.page);
  const [data, me] = await Promise.all([loadTopic(id, page), getMe()]);
  if (data instanceof ApiError) {
    if (data.status === 404) notFound();
    return (
      <>
        <SiteNav />
        <PageError error={data} />
      </>
    );
  }
  if (page > data.total_pages) redirect(topicHref(id, data.total_pages));

  const { topic, forum, posts, users } = data;
  const user = cardLookup(users);
  const mentionable = users.filter((u) => u.id !== me?.id);
  const forums = data.can_moderate ? await listForums() : [];

  return (
    <>
      <SiteNav active={boardHref(forum.slug)} />
      <main className="mx-auto max-w-3xl space-y-3 px-4 py-5">
        <nav aria-label="位置" className="text-sm text-fg-muted">
          <Link href="/" className="hover:text-fg">
            首页
          </Link>{" "}
          /{" "}
          <Link href={boardHref(forum.slug)} className="hover:text-fg">
            {forum.name}
          </Link>
        </nav>
        <div className="flex items-start justify-between gap-3">
          <h1 className="text-2xl leading-snug font-semibold">
            <TopicBadges topic={topic} />
            {topic.title}
          </h1>
          {data.can_moderate ? (
            <TopicModMenu
              topicId={topic.id}
              forumId={forum.id}
              flags={topic.flags}
              forums={forums instanceof ApiError ? [] : forums}
              staff={me?.staff}
            />
          ) : null}
        </div>
        <p className="amount text-sm text-fg-muted">
          {topic.views.toLocaleString("zh-CN")} 浏览 · {topic.replies} 回复 · 第 {page} /{" "}
          {data.total_pages} 页
        </p>
        {posts.map((p) => {
          const own = me?.id === p.author_id;
          return (
            <Floor
              key={p.id}
              post={p}
              author={user(p.author_id)}
              title={p.floor === 1 ? topic.title : undefined}
              mentionable={mentionable}
              mod={
                data.can_moderate ? (
                  <PostModMenu
                    postId={p.id}
                    floor={p.floor}
                    forumId={forum.id}
                    deleted={p.deleted}
                    author={{ id: p.author_id, name: userName(user(p.author_id)) }}
                    staff={me?.staff}
                    isOwn={own}
                  />
                ) : null
              }
              perms={{
                canEdit: !p.deleted && (data.can_moderate || (own && !topic.flags.locked)),
                asModerator: data.can_moderate && !own,
                hasHistory: p.edited_at != null && (!p.deleted || data.can_moderate),
              }}
            />
          );
        })}
        <Pager current={page} total={data.total_pages} href={(p) => topicHref(id, p)} />
        {data.total_pages > 1 ? (
          <form
            action={`/t/${id}`}
            method="get"
            className="flex items-center justify-center gap-2 text-sm whitespace-nowrap"
            aria-label="按楼层跳转"
          >
            <label htmlFor="jump-floor">跳到</label>
            <span className="w-28">
              <Input
                id="jump-floor"
                name="floor"
                type="number"
                inputMode="numeric"
                min={1}
                max={topic.last_floor}
                required
                className="h-8"
                placeholder={`1–${topic.last_floor}`}
              />
            </span>
            <span>楼</span>
            <Button type="submit" size="sm" variant="secondary">
              跳转
            </Button>
          </form>
        ) : null}
        <Card id="reply">
          <h2 className="mb-2 font-medium">回复</h2>
          {data.can_reply ? (
            <ComposerLauncher
              target={{ kind: "reply", topicId: id }}
              label="写回复"
              mentionable={mentionable}
            />
          ) : me === null ? (
            <p className="text-sm text-fg-muted">
              <Link href={loginHref(topicHref(id, page))} className="text-accent hover:underline">
                登录
              </Link>{" "}
              后参与回复
            </p>
          ) : (
            <p className="text-sm text-fg-muted">
              {topic.flags.locked ? "主题已锁定，不能回复" : "你的等级暂时不能在本板块回复"}
            </p>
          )}
        </Card>
      </main>
    </>
  );
}
