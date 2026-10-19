import { ApiError } from "@akari/api-client";
import { Amount, Avatar, Badge, Button, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { cache, type ReactNode } from "react";
import { PresenceDot } from "../../../components/live/PresenceDot";
import { StartConversation } from "../../../components/messages/StartConversation";
import { PageError } from "../../../components/PageError";
import { AppShell } from "../../../components/forum/AppShell";
import { load, serverApi } from "../../../lib/api";
import { formatTime, profileHref, userName } from "../../../lib/display";
import { loginHref, topicHref } from "../../../lib/links";
import { WALLET_ORIGIN } from "../../../lib/public-config";
import { getMe } from "../../../lib/session";

type Params = Promise<{ name: string }>;

/** 路由参数可能仍是 URL 编码形式；本身含 % 的名字解码会失败，原样使用。 */
function decodeParam(raw: string): string {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

const loadProfile = cache(async (user: string) => {
  const api = await serverApi();
  const path = { params: { path: { user } } };
  const [profile, posts] = await Promise.all([
    load(api.GET("/api/v1/users/{user}", path)),
    load(api.GET("/api/v1/users/{user}/posts", path)),
  ]);
  return { profile, posts };
});

export async function generateMetadata({ params }: { params: Params }): Promise<Metadata> {
  const { profile } = await loadProfile(decodeParam((await params).name));
  return profile instanceof ApiError
    ? { robots: { index: false } }
    : { title: userName(profile), robots: { index: false } };
}

/** 成功率（万分比）→「98.6%」。 */
function percent(bps: number): string {
  return `${Math.floor(bps / 100)}.${Math.floor((bps % 100) / 10)}%`;
}

function Stat({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="rounded-md bg-surface-2 px-3 py-2">
      <dt className="text-xs text-fg-muted">{label}</dt>
      <dd className="amount text-lg font-semibold">{value}</dd>
    </div>
  );
}

/**
 * 个人主页：等级与经验、交易信誉（只统计链上最终确认的订单）、最近发帖。资料对所有人相同，游客页可缓存，
 * 在线点在浏览器里取。本人看到账号信息与编辑入口，其他登录用户看到「发私信」。
 */
export default async function ProfilePage({ params }: { params: Params }) {
  const raw = decodeParam((await params).name);
  const [{ profile, posts }, me] = await Promise.all([loadProfile(raw), getMe()]);
  if (profile instanceof ApiError) {
    if (profile.status === 404) notFound();
    return (
      <AppShell>
        <PageError error={profile} />
      </AppShell>
    );
  }
  // 按 ID 打开的、已有用户名的人换成用户名地址：同一个人只有一个规范地址
  if (/^\d+$/.test(raw) && profile.username) redirect(profileHref(profile));

  const name = userName(profile);
  const self = me?.id === profile.id;
  const t = profile.trade;
  const lastActive = profile.last_active_at
    ? profile.last_active_day_only
      ? formatTime(profile.last_active_at).slice(0, 10)
      : formatTime(profile.last_active_at)
    : "—";

  return (
    <AppShell>
      <div className="max-w-4xl">
        <header className="flex flex-col items-center gap-4 border-b border-border pb-5 text-center sm:flex-row sm:items-start sm:text-left">
          <Avatar
            name={name}
            src={profile.avatar_url ?? undefined}
            size={96}
            badge={<PresenceDot userId={profile.id} avatarSize={96} />}
          />
          <div className="min-w-0 flex-1 space-y-1">
            <h1 className="flex flex-wrap items-center justify-center gap-2 text-[1.75rem] leading-snug font-medium sm:justify-start">
              {name}
              <Badge tone="accent">Lv{profile.level}</Badge>
              {self ? <Badge>这是你</Badge> : null}
            </h1>
            <p className="amount text-sm text-fg-muted">
              经验 {profile.exp.toLocaleString("zh-CN")} · {profile.topics} 主题 · {profile.posts}{" "}
              帖子 · 加入于 {formatTime(profile.joined_at).slice(0, 10)} · 最近活跃 {lastActive}
            </p>
            {self ? (
              <p className="text-sm text-fg-muted">登录邮箱：{me.email}（仅自己可见）</p>
            ) : null}
          </div>
          {self ? (
            <div className="flex flex-col gap-2">
              <Button asChild variant="secondary" size="sm">
                <Link href="/settings">{me.username ? "修改用户名" : "设置用户名"}</Link>
              </Button>
              {me.username ? (
                <Button asChild variant="secondary" size="sm">
                  <a href={`${WALLET_ORIGIN}/setup`}>开通交易 / 管理钱包</a>
                </Button>
              ) : null}
            </div>
          ) : me ? (
            <StartConversation userId={profile.id} />
          ) : (
            <Button asChild variant="secondary" size="sm">
              <Link href={loginHref(profileHref(profile))}>登录后发私信</Link>
            </Button>
          )}
        </header>
        {self && !me.username ? (
          <p className="mt-4 rounded-md bg-warning-soft px-3 py-2 text-sm">
            开通交易前需要先{" "}
            <Link href="/settings" className="text-accent underline">
              设置用户名
            </Link>
            ：交易对手和仲裁员都按用户名认人。
          </p>
        ) : null}

        <section aria-labelledby="trade-heading" className="mt-6">
          <h2 id="trade-heading" className="text-lg font-medium">
            交易信誉
          </h2>
          <dl className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-4">
            <Stat
              label="成功率"
              value={t.success_rate_bps != null ? percent(t.success_rate_bps) : "—"}
            />
            <Stat label="成交" value={`${t.success} 笔`} />
            <Stat label="争议败诉" value={`${t.disputes_lost} 次`} />
            <Stat label="成交金额" value={<Amount value={t.volume} />} />
            <Stat label="作为卖家违约" value={`${t.seller_fault_fail} 次`} />
            <Stat label="作为买家违约" value={`${t.buyer_fault_fail} 次`} />
            <Stat label="协商和解" value={`${t.settled} 次`} />
            <Stat
              label="卖家平均回复"
              value={
                t.avg_reply_secs != null
                  ? `${Math.max(1, Math.round(t.avg_reply_secs / 60))} 分钟`
                  : "—"
              }
            />
          </dl>
          <p className="mt-3 text-xs text-fg-muted" data-testid="finalized-note">
            只统计链上已最终确认（finalized）的订单，刚成交的订单要等最终确认后才计入。成功率 = 成功
            ÷（成功 + 本人责任失败），不满 10 笔不显示。
          </p>
        </section>

        <section aria-labelledby="posts-heading" className="mt-6">
          <h2 id="posts-heading" className="border-b border-border pb-2 text-lg font-medium">
            最近发帖
          </h2>
          {posts instanceof ApiError ? (
            <p className="py-6 text-sm text-fg-muted">帖子加载失败：{posts.message}</p>
          ) : posts.posts.length ? (
            <ul>
              {posts.posts.map((p) => (
                <li key={p.id} className="border-b border-border py-3">
                  <Link
                    href={topicHref(p.topic_id, p.page, p.floor)}
                    className="text-[17px] leading-snug hover:text-accent"
                  >
                    {p.floor === 1 ? (
                      <Badge tone="accent" className="mr-2 align-[2px]">
                        主题
                      </Badge>
                    ) : (
                      <Badge className="amount mr-2 align-[2px]">#{p.floor}</Badge>
                    )}
                    {p.topic_title}
                  </Link>
                  <p className="mt-0.5 line-clamp-2 text-sm break-words text-fg-muted">
                    {p.excerpt}
                  </p>
                  <time dateTime={p.created_at} className="text-xs text-fg-muted">
                    {formatTime(p.created_at)}
                  </time>
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState title="还没有发过帖子" />
          )}
        </section>
      </div>
    </AppShell>
  );
}
