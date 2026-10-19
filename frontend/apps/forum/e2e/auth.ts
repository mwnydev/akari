import {
  expect,
  test,
  type Browser,
  type BrowserContext,
  type BrowserContextOptions,
  type Page,
} from "@playwright/test";
import postgres from "postgres";
import { latestCode } from "./mailpit";

/** 与 forum-api 相同的数据库（测试直接造数据、改注册时间）。 */
export function db() {
  return postgres(
    process.env.DATABASE_URL ?? "postgres://akari:akari-dev-only@127.0.0.1:5432/akari",
    { max: 1 },
  );
}

export function uniqueEmail(tag: string) {
  return `e2e-${tag}-${Date.now()}-${Math.floor(Math.random() * 1e6)}@test.dev`;
}

/**
 * 每个测试用户一个假的客户端 IP：登录接口按 IP 限频（每小时 20 次），整套测试的登录次数会超过。
 * forum-api 只信任来自本机的 `CF-Connecting-IP`（生产是 Caddy，本地是 Next 的 rewrite），所以测试里可以这样区分。
 */
export async function useTestIp(page: Page) {
  const n = () => 1 + Math.floor(Math.random() * 254);
  await page.setExtraHTTPHeaders({ "cf-connecting-ip": `10.${n()}.${n()}.${n()}` });
}

/** 走一遍邮箱验证码登录（新邮箱自动注册），停在首页。 */
export async function login(page: Page, email: string) {
  await useTestIp(page);
  await page.goto("/login");
  await page.getByLabel("邮箱").fill(email);
  await page.getByRole("button", { name: "获取验证码" }).click();
  await page.getByLabel("验证码").fill(await latestCode(email));
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page.getByTestId("nav-user")).toBeVisible();
}

/**
 * 把账号的注册时间提前两天：注册 24 小时内发帖限频每小时 5 条（plan 第 9 节），
 * 测试要造满一页以上的楼层。直接改与 forum-api 相同的数据库（DATABASE_URL）。
 */
export async function backdateAccount(email: string) {
  const sql = db();
  try {
    const rows =
      await sql`UPDATE users SET created_at = now() - interval '2 days' WHERE email = ${email} RETURNING id`;
    expect(rows).toHaveLength(1);
  } finally {
    await sql.end();
  }
}

export interface Member {
  context: BrowserContext;
  page: Page;
  email: string;
  id: number;
  /** 设置了用户名时 */
  username: string | null;
}

/**
 * 独立的浏览器上下文里登录一个新用户（设备参数与当前项目一致：手机项目就是手机视口）；
 * `username` 为真时经同源 API 设好用户名。
 */
export async function member(
  browser: Browser,
  baseURL: string,
  tag: string,
  opts: { username?: boolean; ignoreHTTPSErrors?: boolean } = {},
): Promise<Member> {
  const { viewport, userAgent, deviceScaleFactor, isMobile, hasTouch } = test.info().project.use;
  const device = { viewport, userAgent, deviceScaleFactor, isMobile, hasTouch };
  const context = await browser.newContext({
    ...(Object.fromEntries(
      Object.entries(device).filter(([, v]) => v !== undefined),
    ) as BrowserContextOptions),
    baseURL,
    ignoreHTTPSErrors: opts.ignoreHTTPSErrors ?? false,
  });
  const page = await context.newPage();
  const email = uniqueEmail(tag);
  await login(page, email);
  const me = (await (await page.request.get("/api/v1/me")).json()) as { id: number };
  let username: string | null = null;
  if (opts.username) {
    username = uniqueName(tag);
    // 相对地址：请求发到页面同源，经 Next rewrite 到同一个 forum-api（见 e2e/README.md「会话掉线」）
    const res = await page.request.put("/api/v1/me/username", {
      headers: { origin: new URL(baseURL).origin },
      data: { username },
    });
    expect(res.status(), await res.text()).toBe(200);
  }
  return { context, page, email, id: me.id, username };
}

/** 合法且基本不会撞名的用户名（≤ 20 个字符，字母开头）。 */
export function uniqueName(tag: string): string {
  const t = tag.replace(/[^A-Za-z]/g, "").slice(0, 6) || "u";
  return `${t}${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`.slice(0, 20);
}
