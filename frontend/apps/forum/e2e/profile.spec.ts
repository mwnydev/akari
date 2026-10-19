import { expect, test } from "@playwright/test";
import { login, member, uniqueEmail, uniqueName } from "./auth";

test("member sets a username in settings, keeps the session, and sees the profile", async ({
  page,
  browser,
  baseURL,
}, info) => {
  test.setTimeout(60_000);
  await login(page, uniqueEmail(`name-${info.project.name}`));
  const name = uniqueName(`n${info.project.name}`);

  await page.getByTestId("nav-user").click();
  await page.getByRole("link", { name: "账号设置" }).click();
  await expect(page).toHaveURL(/\/settings$/);
  // 菜单里是普通链接（整页跳转）：等页面脚本就绪再填表
  await page.waitForLoadState();
  const form = page.getByRole("form", { name: "用户名" });
  const field = form.getByRole("textbox", { name: "用户名" });
  const save = form.getByRole("button", { name: "保存" });

  // 本地检查与后端规则（保留名）都给出中文提示
  await field.fill("12345");
  await save.click();
  await expect(form.getByRole("alert")).toContainText("纯数字");
  await field.fill("admin");
  await save.click();
  await expect(form.getByRole("alert")).toContainText("保留名");

  // 保存成功：导航栏立即换成新名字；刷新后仍是登录状态（真实浏览器流程里会话不会掉）
  await field.fill(name);
  await save.click();
  await expect(form.getByRole("status")).toContainText("已保存");
  await expect(page.getByTestId("nav-user")).toHaveAccessibleName(new RegExp(name));
  await page.reload();
  await expect(page.getByTestId("nav-user")).toHaveAccessibleName(new RegExp(name));
  expect((await page.request.get("/api/v1/me")).status()).toBe(200);

  // 30 天内不能再改
  await field.fill(`${name.slice(0, 18)}x`);
  await save.click();
  await expect(form.getByRole("alert")).toContainText("30 天");

  // 别人（不区分大小写）用不了这个名字
  const other = await member(browser, baseURL ?? "", `taken${info.project.name}`);
  try {
    await other.page.goto("/settings");
    await other.page.getByRole("textbox", { name: "用户名" }).fill(name.toUpperCase());
    await other.page.getByRole("button", { name: "保存" }).click();
    await expect(other.page.getByRole("form", { name: "用户名" }).getByRole("alert")).toContainText(
      "已被使用",
    );
  } finally {
    await other.context.close();
  }

  // 个人主页：等级、经验、交易信誉（只认最终确认的订单）、最近发帖
  await page.getByTestId("nav-user").click();
  await page.getByRole("link", { name: "个人主页" }).click();
  await expect(page).toHaveURL(new RegExp(`/u/${name}$`));
  await expect(page.getByRole("heading", { level: 1 })).toContainText(name);
  await expect(page.getByRole("heading", { level: 1 })).toContainText("Lv1");
  await expect(page.getByText(/经验 \d+/)).toBeVisible();
  await expect(page.getByTestId("finalized-note")).toContainText("最终确认");
  await expect(page.getByText("还没有发过帖子")).toBeVisible();

  // 游客看到同一个主页（可缓存），没有本人才有的信息
  const guest = await browser.newContext({ baseURL: baseURL ?? "" });
  try {
    const gp = await guest.newPage();
    const res = await gp.goto(`/u/${name}`);
    expect(res?.headers()["cloudflare-cdn-cache-control"]).toBe("public, max-age=60");
    await expect(gp.getByRole("heading", { level: 1 })).toContainText(name);
    await expect(gp.getByText("仅自己可见")).toHaveCount(0);
    await expect(gp.getByRole("link", { name: "登录后发私信" })).toBeVisible();
  } finally {
    await guest.close();
  }
});
