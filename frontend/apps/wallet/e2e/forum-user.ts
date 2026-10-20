import type { BrowserContext } from "@playwright/test";
import { execFileSync } from "node:child_process";

// 真实 forum-api 上的测试用户：邮箱验证码登录（Mailpit 截获邮件），再直接在数据库里设用户名（开通交易要求有用户名）
// 与测试版块（允许交易帖；还没有建版块的接口）。需要 forum-worker 在跑，否则收不到邮件。

const MAILPIT = process.env.MAILPIT_URL ?? "http://127.0.0.1:8025";
const PG_CONTAINER = process.env.WALLET_E2E_PG_CONTAINER ?? "akari-dev-postgres-1";
const PG_DATABASE = process.env.WALLET_E2E_DATABASE ?? "akari";

interface Mail {
  id: string;
  code: string;
}

/** 发给 `to` 的最新一封带 6 位验证码的邮件；`after` 之前（含）的邮件不算。 */
export async function waitForCode(to: string, after?: Mail, timeoutMs = 15_000): Promise<Mail> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const res = await fetch(`${MAILPIT}/api/v1/search?query=${encodeURIComponent(`to:${to}`)}`);
    const { messages } = (await res.json()) as { messages: { ID: string }[] };
    const id = messages[0]?.ID;
    if (id && id !== after?.id) {
      const msg = (await (await fetch(`${MAILPIT}/api/v1/message/${id}`)).json()) as {
        Text: string;
      };
      const code = /\b(\d{6})\b/.exec(msg.Text)?.[1];
      if (code) return { id, code };
    }
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`no code mailed to ${to}`);
}

/** 用钱包页面的源（在 WALLET_ORIGINS 里）登录；会话 Cookie 落在这个浏览器上下文里。 */
export async function signIn(context: BrowserContext, baseURL: string, tag: string) {
  const email = `wallet-e2e-${tag}-${Date.now()}@test.dev`;
  const headers = { origin: baseURL };
  const start = await context.request.post(`${baseURL}/api/v1/auth/email/start`, {
    headers,
    data: { email, captcha: null },
  });
  if (!start.ok()) throw new Error(`login start failed: ${start.status()}`);
  const mail = await waitForCode(email);
  const verify = await context.request.post(`${baseURL}/api/v1/auth/email/verify`, {
    headers,
    data: { email, code: mail.code },
  });
  if (!verify.ok()) throw new Error(`login verify failed: ${verify.status()}`);
  const username = `w${tag.replace(/\W/g, "").slice(0, 10)}${Date.now() % 1e9}`;
  psql("UPDATE users SET username = :'username' WHERE email = :'email';", { username, email });
  return { email, loginMail: mail };
}

/** 测试用的交易版块（新用户也能发帖、允许交易帖与拍卖帖），不存在就建；返回 slug。 */
export function ensureBoard(): string {
  const slug = "e2e";
  psql(
    "INSERT INTO forums (slug, name, post_min_level, allow_trade, allow_auction) VALUES (:'slug', 'e2e', 0, true, true) ON CONFLICT (slug) DO UPDATE SET allow_trade = true, allow_auction = true;",
    { slug },
  );
  return slug;
}

/**
 * 真实 forum-sponsor 只代付库里当前那份成交证明（policy：授权哈希、赢家、价格、截止时间、平台签名都要对上）。
 * 测试的拍卖由 fake-backend 回答，这里把 forum-worker 与 forum-sponsor 本该写入的那一份（待付款的拍卖、出价、第 1 份
 * 成交证明）直接写进测试库；卖家与出价人用库里任意一个用户（代付只看付款人是不是证明里的赢家）。十六进制都不带 0x。
 */
export function seedSettledAuction(p: {
  chainId: number;
  escrow: string;
  seller: string;
  authHash: string;
  winner: string;
  price: bigint;
  amount: bigint;
  payDeadline: number;
  signature: string;
}) {
  const hex = (v: string) => v.replace(/^0x/, "");
  psql(
    `WITH u AS (SELECT min(id) AS id FROM users), f AS (SELECT min(id) AS id FROM forums),
     t AS (INSERT INTO topics (forum_id, author_id, title, kind, last_post_at)
           SELECT f.id, u.id, 'e2e 拍卖', 3, now() FROM f, u RETURNING id, forum_id, author_id),
     a AS (INSERT INTO auctions (seller_id, forum_id, topic_id, chain_id, verifying_contract, seller, currency,
             cny_per_usdt, start_price, increment, reserve_price, duration_secs, terms, content_hash, auth, auth_hash,
             seller_signature, state, round, starts_at, ends_at, max_ends_at, pay_deadline)
           SELECT t.author_id, t.forum_id, t.id, :'chain'::bigint, decode(:'escrow', 'hex'), decode(:'seller', 'hex'),
             1, 72000, 50000, 1000, 80000, 259200, '{}', sha256(:'auth_hash'::bytea), '{}', decode(:'auth_hash', 'hex'),
             '\\x01', 2, 1, now() - interval '3 days', now() - interval '1 minute', now() + interval '1 day',
             to_timestamp(:'deadline'::bigint)
           FROM t RETURNING id, seller_id),
     b AS (INSERT INTO auction_bids (auction_id, bidder_id, amount, amount_usdt)
           SELECT a.id, a.seller_id, :'price'::numeric, :'amount'::numeric FROM a RETURNING id, auction_id, bidder_id)
     INSERT INTO auction_proofs (auction_id, round, bid_id, bidder_id, auth_hash, winner, price, pay_deadline, signature)
     SELECT b.auction_id, 1, b.id, b.bidder_id, decode(:'auth_hash', 'hex'), decode(:'winner', 'hex'),
       :'price'::numeric, to_timestamp(:'deadline'::bigint), decode(:'signature', 'hex')
     FROM b;`,
    {
      chain: String(p.chainId),
      escrow: hex(p.escrow),
      seller: hex(p.seller),
      auth_hash: hex(p.authHash),
      winner: hex(p.winner),
      price: p.price.toString(),
      amount: p.amount.toString(),
      deadline: String(p.payDeadline),
      signature: hex(p.signature),
    },
  );
}

/** 在测试数据库上执行一段 SQL；psql 变量只在标准输入的脚本里展开（-c 不展开），用 :'x' 按字面量引用，避免拼 SQL。 */
function psql(sql: string, vars: Record<string, string>) {
  execFileSync(
    "docker",
    [
      "exec",
      "-i",
      PG_CONTAINER,
      "psql",
      "-U",
      "akari",
      "-d",
      PG_DATABASE,
      "-v",
      "ON_ERROR_STOP=1",
      ...Object.entries(vars).flatMap(([k, v]) => ["-v", `${k}=${v}`]),
    ],
    { input: sql },
  );
}
