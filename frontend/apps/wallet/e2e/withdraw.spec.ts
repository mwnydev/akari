import { localChain as chain } from "@akari/contracts";
import { expect, test } from "@playwright/test";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { chainIsDown } from "./env";
import { U, balances, confirmSignature, createWallet, harness, mintUsdt } from "./harness";

// 钱包首页：真实余额、充值地址、提币地址管理（Passkey 确认、冷却、撤销）、「全部提回」= 余额 − 代付费，
// 经 ERC-7677 代付一次 Face ID 转到已生效的提币地址。提币地址与余额接口没上线时由模拟后端提供（./fake-backend.ts）。

test("保存提币地址 → 全部提回交易所 → 余额刷新", async ({ browser, baseURL }) => {
  const down = await chainIsDown();
  test.skip(down !== null, down ?? "");
  const h = await harness();
  const user = await h.newUser(browser, "withdraw", baseURL ?? "");
  const wallet = await createWallet(user);
  await mintUsdt(wallet, U("30"));
  const { page } = user;

  await page.goto("/");
  await expect(page.getByTestId("balance-usdt")).toContainText("30.00");
  await expect(page.getByTestId("deposit-address")).toHaveText(wallet);
  await expect(page.getByTestId("deposit-qr").locator("path")).toHaveCount(1);
  await expect(page.getByText("选 TRC20 无法找回")).toBeVisible();

  // 地址校验：自己的钱包、合约地址都不行
  const address = page.getByLabel("充值地址（Arbitrum One）");
  await address.fill(wallet);
  await expect(page.getByText("这是你自己的钱包地址")).toBeVisible();
  await address.fill(chain.contracts.MockUSDT);
  await expect(page.getByText("这是合约地址")).toBeVisible();

  // 第一个地址立即生效；注册 Passkey 刚算过近期验证，这里不再弹 Face ID
  const exchange = privateKeyToAccount(generatePrivateKey()).address;
  await page.getByLabel("备注").fill("币安");
  await address.fill(exchange);
  await page.getByRole("button", { name: "保存地址（Face ID 确认）" }).click();
  const rows = page.getByTestId("withdraw-address");
  await expect(rows.filter({ hasText: "币安" })).toContainText("可用");

  // 第二个地址进入 24 小时冷却，可以撤销
  const other = privateKeyToAccount(generatePrivateKey()).address;
  await page.getByLabel("备注").fill("OKX");
  await address.fill(other);
  await page.getByRole("button", { name: "保存地址（Face ID 确认）" }).click();
  const okx = rows.filter({ hasText: "OKX" });
  await expect(okx).toContainText("冷却中");
  await okx.getByRole("button", { name: "撤销" }).click();
  await okx.getByRole("button", { name: "确认撤销（Face ID）" }).click();
  await expect(okx).toHaveCount(0);

  // 全部提回：30 − 0.05 代付费 = 29.95 到账，钱包共扣 30（用掉开通送的 1 次免费代付提币）
  await expect(page.getByTestId("withdraw-credits")).toHaveText(
    "免费代付提币剩余 1 次（今日还可 3 次）",
  );
  await page.getByRole("button", { name: "全部提回" }).click();
  await expect(page.getByLabel("到账金额（USDT）")).toHaveValue("29.95");
  const signs = await user.signCount();
  await page.getByRole("button", { name: "提回交易所", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "确认提回交易所" });
  await expect(dialog).toContainText("29.95");
  await expect(dialog).toContainText("0.05");
  await confirmSignature(page, "提回交易所", "30.00", exchange);
  expect(await user.signCount()).toBe(signs + 1);
  await expect(page.getByTestId("withdraw-done")).toContainText("29.95");
  await expect(page.getByTestId("balance-usdt")).toContainText("0.00");
  // 次数用完：提交按钮禁用，引导去完成订单或紧急取回
  await expect(page.getByTestId("withdraw-credits")).toContainText("剩余 0 次");
  await expect(page.getByText("完成一笔担保交易后可再获得 1 次")).toBeVisible();
  await expect(page.getByRole("link", { name: "「紧急取回」" })).toHaveAttribute(
    "href",
    "/emergency",
  );
  await expect(page.getByRole("button", { name: "提回交易所", exact: true })).toBeDisabled();

  expect(await balances({ wallet, exchange })).toEqual({ wallet: 0n, exchange: U("29.95") });
});
