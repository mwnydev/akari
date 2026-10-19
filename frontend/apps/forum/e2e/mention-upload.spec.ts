import { expect, test } from "@playwright/test";
import { crc32, deflateSync } from "node:zlib";
import { member } from "./auth";

/** 纯色 PNG（测试图片，不用仓库里放二进制文件）。 */
function png(width: number, height: number): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, 2, 0, 0, 0], 8); // 8 位 RGB
  const row = Buffer.concat([Buffer.from([0]), Buffer.alloc(width * 3, 0x5a)]);
  const raw = Buffer.concat(Array.from({ length: height }, () => row));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

test("composer: @mention autocomplete from user search, mention links and live notification; image upload to WebP", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const base = baseURL ?? "";
  const mentioned = await member(browser, base, `men${info.project.name}`, { username: true });
  // 图片的公开地址在本地是自签名 HTTPS（见 e2e/README.md）
  const author = await member(browser, base, `aut${info.project.name}`, {
    ignoreHTTPSErrors: true,
  });
  const name = mentioned.username ?? "";
  try {
    await mentioned.page.goto("/");
    const page = author.page;
    await page.goto("/f/general");
    await page.getByRole("button", { name: "发主题" }).click();
    await page.getByRole("textbox", { name: "标题" }).fill(`提及与图片 ${Date.now()}`);
    const body = page.getByRole("textbox", { name: "正文" });

    // 不在本页的人：输入 @ 加前缀，从用户名搜索接口给出候选
    await body.pressSequentially(`请看 @${name.slice(0, -3)}`);
    const option = page.getByRole("option", { name: `@${name}` });
    await expect(option).toBeVisible();
    await expect(option).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Enter");
    await body.pressSequentially("这张图");

    // 图片：本地先查类型与大小，再直传、等转换成 WebP 后插入
    const input = page.getByTestId("image-input");
    await input.setInputFiles({
      name: "notes.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("x"),
    });
    await expect(page.getByRole("form", { name: "发布主题" }).getByRole("alert")).toContainText(
      "只支持",
    );
    await input.setInputFiles({ name: "photo.png", mimeType: "image/png", buffer: png(400, 300) });
    const inserted = body.locator('img[src$=".webp"]');
    await expect(inserted).toBeVisible({ timeout: 30_000 });
    await expect
      .poll(() => inserted.evaluate((img: HTMLImageElement) => img.naturalWidth))
      .toBeGreaterThan(0);

    await page.getByRole("button", { name: "发布主题" }).click();
    await expect(page).toHaveURL(/\/t\/\d+$/);
    const floor = page.locator("#floor-1");
    const link = floor.getByRole("link", { name: `@${name}` });
    await expect(link).toHaveAttribute("href", `/u/${mentioned.id}`);
    await expect(floor.locator('img[src$=".webp"]')).toBeVisible();

    // 被提及的人停在首页：通知角标实时出现；通知链回这一楼；提及链接打开对方主页
    await expect(mentioned.page.getByTestId("unread-badge")).toHaveText("1", { timeout: 15_000 });
    await mentioned.page.goto("/notifications");
    const note = mentioned.page.getByTestId("notification").first();
    await expect(note).toContainText("在 #1 楼提到了你");
    await link.click();
    await expect(page).toHaveURL(new RegExp(`/u/${name}$`));
  } finally {
    await mentioned.context.close();
    await author.context.close();
  }
});
