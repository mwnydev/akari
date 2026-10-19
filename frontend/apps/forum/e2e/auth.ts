import { expect, type Page } from "@playwright/test";
import postgres from "postgres";
import { latestCode } from "./mailpit";

export function uniqueEmail(tag: string) {
  return `e2e-${tag}-${Date.now()}-${Math.floor(Math.random() * 1e6)}@test.dev`;
}

/** 走一遍邮箱验证码登录（新邮箱自动注册），停在首页。 */
export async function login(page: Page, email: string) {
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
  const sql = postgres(
    process.env.DATABASE_URL ?? "postgres://akari:akari-dev-only@127.0.0.1:5432/akari",
    { max: 1 },
  );
  try {
    const rows =
      await sql`UPDATE users SET created_at = now() - interval '2 days' WHERE email = ${email} RETURNING id`;
    expect(rows).toHaveLength(1);
  } finally {
    await sql.end();
  }
}
