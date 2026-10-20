import { expect, test } from "@playwright/test";
import { uniqueEmail, useTestIp } from "./auth";
import { latestCode } from "./mailpit";
import { GUEST_EDGE_CACHE } from "../lib/edge-cache";

test("guest logs in with an emailed code, sees themselves in the nav, then logs out", async ({
  page,
}, info) => {
  const email = uniqueEmail(info.project.name);
  await useTestIp(page);

  await page.goto("/");
  await page.getByRole("link", { name: "登录", exact: true }).click();
  await expect(page).toHaveURL(/\/login/);

  await page.getByLabel("邮箱").fill(email);
  await page.getByRole("button", { name: "获取验证码" }).click();
  await expect(page.getByText(`验证码已发送到`)).toBeVisible();
  await expect(page.getByRole("button", { name: /秒后可重发/ })).toBeDisabled();

  await page.getByLabel("验证码").fill(await latestCode(email));
  await page.getByRole("button", { name: "登录", exact: true }).click();

  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByTestId("nav-user")).toBeVisible();

  // 新账号：弹一次「设置用户名」，「以后再说」后不再主动弹（交易入口还会再问）
  const prompt = page.getByRole("dialog", { name: "给自己起个用户名" });
  await expect(prompt).toBeVisible();
  await prompt.getByRole("button", { name: "以后再说" }).click();
  await expect(prompt).toBeHidden();
  await page.reload();
  await expect(page.getByTestId("nav-user")).toBeVisible();
  await page.waitForLoadState("networkidle");
  await expect(prompt).toBeHidden();

  // 头像打开账号菜单
  await page.getByTestId("nav-user").click();
  await page.getByRole("button", { name: "退出" }).click();
  await expect(page.getByRole("link", { name: "登录", exact: true })).toBeVisible();
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
  expect(res?.headers()["cloudflare-cdn-cache-control"]).toBe(GUEST_EDGE_CACHE);
  await expect(page.getByRole("link", { name: "登录", exact: true })).toBeVisible();
});

const WALLET = process.env.WALLET_ORIGIN ?? "http://localhost:3001";

test("login returns to the wallet only for the configured wallet origin", async ({
  page,
}, info) => {
  // 钱包不在跑：拦下对钱包源的请求，只看登录后跳到了哪里
  await page.route(`${WALLET}/**`, (route) =>
    route.fulfill({ contentType: "text/html", body: "<title>wallet</title>wallet" }),
  );
  const email = uniqueEmail(`${info.project.name}-next`);
  await useTestIp(page);
  await page.goto(`/login?next=${encodeURIComponent(`${WALLET}/order?id=5`)}`);
  await page.getByLabel("邮箱").fill(email);
  await page.getByRole("button", { name: "获取验证码" }).click();
  await page.getByLabel("验证码").fill(await latestCode(email));
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await page.waitForURL(`${WALLET}/order?id=5`);

  // 已登录时直接跳；别的源一律回首页（防开放重定向）
  // （服务端 307 跳转后的请求不经过 page.route，钱包没在跑时导航本身会失败，只核对跳去的地址）
  const [redirected] = await Promise.all([
    page.waitForRequest(`${WALLET}/setup`),
    page.goto(`/login?next=${encodeURIComponent(`${WALLET}/setup`)}`).catch(() => null),
  ]);
  expect(redirected.url()).toBe(`${WALLET}/setup`);
  for (const evil of ["https://evil.example/", `${WALLET}.evil.example/`, "//evil.example/"]) {
    await page.goto(`/login?next=${encodeURIComponent(evil)}`);
    await expect(page).toHaveURL(/^http:\/\/localhost:\d+\/$/);
  }
});
