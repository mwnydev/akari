import { expect, type Page } from "@playwright/test";
import { db } from "./auth";
import { latestCode } from "./mailpit";

// 管理人员端到端测试的公共部分：虚拟 Passkey、专用邮箱验证码、直接写库造角色与争议订单。

/** 后台静态导出的地址（须与 forum-api 的 ADMIN_ORIGIN 一致）。 */
export const ADMIN_URL = process.env.ADMIN_URL ?? "http://localhost:3002";
const MAILPIT = process.env.MAILPIT_URL ?? "http://127.0.0.1:8025";

/** Chromium 的虚拟 WebAuthn 认证器（CDP）：自动按下、通过用户验证，可发现凭证。 */
export async function virtualAuthenticator(page: Page): Promise<() => Promise<number>> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("WebAuthn.enable");
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  // 返回「这个认证器里有几个凭证」，用来确认用的是哪一组 Passkey
  return async () =>
    (await cdp.send("WebAuthn.getCredentials", { authenticatorId })).credentials.length;
}

/** 先清掉这个邮箱之前的邮件，再触发发码，取新发来的验证码（登录码与各种专用码不会混淆）。 */
export async function freshCode(email: string, trigger: () => Promise<void>): Promise<string> {
  await fetch(`${MAILPIT}/api/v1/search?query=${encodeURIComponent(`to:${email}`)}`, {
    method: "DELETE",
  });
  await trigger();
  return latestCode(email);
}

/** 1 = 站长、3 = 管理员、4 = 版主（须给板块）。 */
export async function grantRole(userId: number, role: 1 | 3 | 4, forumId: number | null = null) {
  const sql = db();
  try {
    await sql`INSERT INTO staff_roles (user_id, role, forum_id) VALUES (${userId}, ${role}, ${forumId})`;
  } finally {
    await sql.end();
  }
}

export async function forumId(slug: string): Promise<number> {
  const sql = db();
  try {
    const [row] = await sql<{ id: number }[]>`SELECT id FROM forums WHERE slug = ${slug}`;
    if (!row) throw new Error(`no forum ${slug}（先导入 backend/crates/db/seeds/dev.sql）`);
    return row.id;
  } finally {
    await sql.end();
  }
}

/** 经同源 API 发一个主题（写请求必须带白名单 Origin）。 */
export async function createTopic(page: Page, baseURL: string, slug: string, title: string) {
  const res = await page.request.post(`/api/v1/forums/${slug}/topics`, {
    headers: { origin: new URL(baseURL).origin },
    data: { title, html: `<p>${title} 的正文</p>`, source: { type: "doc" } },
  });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { topic_id: number }).topic_id;
}

const CHAIN_ID = 421_614;

/**
 * 争议中的一次性订单（`usdt` 整 USDT，< 200 一人裁决）与它的案件，并把案件分给 `arbitrator`。
 * 订单与案件本来由 indexer 写入、仲裁员由 forum-worker 随机分配；这里一条语句写完，worker 不会插进来另行分配。
 */
export async function seedDispute(
  buyer: number,
  seller: number,
  arbitrator: number,
  usdt: number,
): Promise<{ orderId: number; caseId: number }> {
  const sql = db();
  try {
    await sql`INSERT INTO chains (chain_id, name, escrow_address, stream_address, config_address,
                token_address, token_symbol, token_decimals, start_block, safe_depth, finalized_depth)
              VALUES (${CHAIN_ID}, 'e2e', decode(repeat('44', 20), 'hex'), decode(repeat('5e', 20), 'hex'),
                decode(repeat('c0', 20), 'hex'), decode(repeat('33', 20), 'hex'), 'USDT', 6, 0, 240, 4800)
              ON CONFLICT (chain_id) DO NOTHING`;
    const chainOrderId = Date.now() * 1000 + Math.floor(Math.random() * 1000);
    const [row] = await sql<{ order_id: number; case_id: number }[]>`
      WITH o AS (
        INSERT INTO escrow_orders (chain_id, contract, chain_order_id, mode, buyer, seller, buyer_id, seller_id,
          token, amount, instant, offer_hash, listing_hash, state, disputed_at, level, created_block, updated_block)
        VALUES (${CHAIN_ID}, decode(repeat('44', 20), 'hex'), ${chainOrderId}, 0, decode(repeat('b1', 20), 'hex'),
          decode(repeat('c1', 20), 'hex'), ${buyer}, ${seller}, decode(repeat('33', 20), 'hex'),
          ${usdt}::bigint * 1000000, false, sha256(${String(chainOrderId)}::bytea), decode(repeat('ab', 32), 'hex'),
          2, now() - interval '1 hour', 'safe', 1, 1)
        RETURNING id
      ), c AS (
        INSERT INTO arbitration_cases (order_id, panel_size, evidence_until)
        SELECT id, 1, now() + interval '71 hours' FROM o
        RETURNING id, order_id
      ), a AS (
        INSERT INTO arbitration_assignments (case_id, arbitrator_id, role, due_at)
        SELECT id, ${arbitrator}, 'panel', now() + interval '5 days' FROM c
      )
      SELECT order_id::int AS order_id, id::int AS case_id FROM c`;
    if (!row) throw new Error("dispute not created");
    return { orderId: row.order_id, caseId: row.case_id };
  } finally {
    await sql.end();
  }
}

/** 举证期提前结束（真实流程要等 72 小时）。 */
export async function endEvidence(caseId: number) {
  const sql = db();
  try {
    await sql`UPDATE arbitration_cases SET evidence_until = now() - interval '1 minute' WHERE id = ${caseId}`;
  } finally {
    await sql.end();
  }
}
