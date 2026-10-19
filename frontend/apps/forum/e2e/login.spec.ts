import { expect, test } from "@playwright/test";
import { uniqueEmail, useTestIp } from "./auth";
import { latestCode } from "./mailpit";

test("guest logs in with an emailed code, sees themselves in the nav, then logs out", async ({
  page,
}, info) => {
  const email = uniqueEmail(info.project.name);
  await useTestIp(page);

  await page.goto("/");
  await page.getByRole("link", { name: "登录" }).click();
  await expect(page).toHaveURL(/\/login/);

  await page.getByLabel("邮箱").fill(email);
  await page.getByRole("button", { name: "获取验证码" }).click();
  await expect(page.getByText(`验证码已发送到`)).toBeVisible();
  await expect(page.getByRole("button", { name: /秒后可重发/ })).toBeDisabled();

  await page.getByLabel("验证码").fill(await latestCode(email));
  await page.getByRole("button", { name: "登录", exact: true }).click();

  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByTestId("nav-user")).toBeVisible();

  await page.getByRole("button", { name: "退出" }).click();
  await expect(page.getByRole("link", { name: "登录" })).toBeVisible();
});

test("wrong code and bad email show Chinese errors", async ({ page }, info) => {
  await useTestIp(page);
  await page.goto("/login");
  await page.getByLabel("邮箱").fill("not-an-email");
  await page.getByRole("button", { name: "获取验证码" }).click();
  await expect(page.getByText("请输入正确的邮箱地址")).toBeVisible();

  const email = uniqueEmail(`${info.project.name}-bad`);
  await page.getByLabel("邮箱").fill(email);
  await page.getByRole("button", { name: "获取验证码" }).click();
  const real = await latestCode(email);
  const wrong = real === "000000" ? "111111" : "000000";
  await page.getByLabel("验证码").fill(wrong);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page.getByText("验证码错误或已过期")).toBeVisible();
});

test("a forged session cookie renders the guest page", async ({ page, context, baseURL }) => {
  await context.addCookies([
    { name: "sid", value: `${"A".repeat(43)}=forged`, url: baseURL ?? "http://localhost:3000" },
  ]);
  const res = await page.goto("/");
  expect(res?.headers()["cloudflare-cdn-cache-control"]).toBe("public, max-age=60");
  await expect(page.getByRole("link", { name: "登录" })).toBeVisible();
});
