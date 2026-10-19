import type { BrowserContext } from "@playwright/test";
import { execFileSync } from "node:child_process";

// 真实 forum-api 上的测试用户：邮箱验证码登录（Mailpit 截获邮件），再直接在数据库里设用户名
// （开通交易要求有用户名，设置用户名的接口还没有）与测试版块。需要 forum-worker 在跑，否则收不到邮件。

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
  const username = `w${tag}${Date.now() % 1e9}`;
  psql("UPDATE users SET username = :'username' WHERE email = :'email';", { username, email });
  return { email, loginMail: mail };
}

/** 测试用的交易版块（新用户也能发帖），不存在就建；返回 slug。 */
export function ensureBoard(): string {
  const slug = "e2e";
  psql(
    "INSERT INTO forums (slug, name, post_min_level) VALUES (:'slug', 'e2e', 0) ON CONFLICT (slug) DO NOTHING;",
    { slug },
  );
  return slug;
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
