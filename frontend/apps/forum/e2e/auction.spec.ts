import { expect, test, type Browser, type Page } from "@playwright/test";
import { db, member, type Member } from "./auth";

// 拍卖帖（真实 forum-api 与 forum-worker，见 e2e/README.md）：出价、被超价推送、防狙击顺延、结束后的各状态、列表信息行、
// 发起拍卖到钱包签名的交接。拍卖上线要卖家在钱包签授权并经链上验签（forum-api 的 auction_e2e.rs 与钱包的端到端测试
// 覆盖），这里先经 API 发普通主题，再直接在库里把它变成上线中的拍卖；钱包也直接写库（与 trade-page.spec 同一思路）。

const BOARD = "auction";
const HOUR = 3_600_000;
const DAY = 86_400;

type Sql = ReturnType<typeof db>;

async function withDb<T>(f: (sql: Sql) => Promise<T>): Promise<T> {
  const sql = db();
  try {
    return await f(sql);
  } finally {
    await sql.end();
  }
}

const origin = () => new URL(test.info().project.use.baseURL ?? "").origin;

/** 站长后台的拍卖总开关（默认关闭）。 */
async function enableAuctions() {
  await withDb(
    (
      sql,
    ) => sql`INSERT INTO site_settings (key, value) VALUES ('auction', ${sql.json({ enabled: true })})
      ON CONFLICT (key) DO UPDATE SET value = site_settings.value || ${sql.json({ enabled: true })}`,
  );
}

/** forum-api 配置的链（钱包与拍卖都登记在它上面）。 */
async function chainId(m: Member): Promise<number> {
  const res = await m.page.request.get("/api/v1/trade/params");
  expect(res.status(), await res.text()).toBe(200);
  return ((await res.json()) as { domain: { chainId: number } }).domain.chainId;
}

/**
 * 开通交易：钱包 Passkey 与钱包地址直接写库（真实流程见钱包的端到端测试）；注册时间提前一个月，
 * 绕开新账号单场出价上限。
 */
async function trader(m: Member, chain: number) {
  const seed = `e2e-wallet-${m.id}`;
  await withDb(async (sql) => {
    await sql`UPDATE users SET created_at = now() - interval '30 days' WHERE id = ${m.id}`;
    await sql`WITH pk AS (
        INSERT INTO passkeys (user_id, purpose, credential_id, credential, public_key_cose, pubkey_x, pubkey_y)
        VALUES (${m.id}, 1, sha256(${`${seed}:id`}::bytea), '{}', '\\x00',
                sha256(${`${seed}:x`}::bytea), sha256(${`${seed}:y`}::bytea))
        RETURNING id)
      INSERT INTO wallets (user_id, chain_id, address, passkey_id)
      SELECT ${m.id}, ${chain}, substring(sha256(${seed}::bytea) from 1 for 20), id FROM pk`;
  });
}

interface Seeded {
  topicId: number;
  auctionId: number;
  title: string;
}

/** 上线中的拍卖帖：人民币（汇率 7.2）或 USDT 计价，金额为拍卖币种最小单位；结束时间可指定。 */
async function liveAuction(
  seller: Member,
  chain: number,
  o: {
    currency: "cny" | "usdt";
    start: number;
    increment: number;
    reserve: number;
    endsInMs: number;
  },
): Promise<Seeded> {
  const title = `拍卖测试 ${test.info().project.name} ${Date.now()}`;
  const res = await seller.page.request.post(`/api/v1/forums/${BOARD}/topics`, {
    headers: { origin: origin() },
    data: { title, html: "<p>未拆封，带原盒。</p>", source: { type: "doc" } },
  });
  expect(res.status(), await res.text()).toBe(201);
  const { topic_id: topicId } = (await res.json()) as { topic_id: number };
  const ends = new Date(Date.now() + o.endsInMs);
  const auctionId = await withDb(async (sql) => {
    await sql`UPDATE topics SET kind = 3 WHERE id = ${topicId}`;
    const [row] = await sql<{ id: number }[]>`
      INSERT INTO auctions (seller_id, forum_id, topic_id, chain_id, verifying_contract, seller, currency,
        cny_per_usdt, start_price, increment, reserve_price, duration_secs, terms, content_hash, auth, auth_hash,
        seller_signature, state, starts_at, ends_at, max_ends_at, snipe_window_secs, snipe_extension_secs,
        extension_cap_secs)
      SELECT ${seller.id}, t.forum_id, t.id, ${chain}, decode(repeat('44', 20), 'hex'), w.address,
        ${o.currency === "cny" ? 1 : 0}, ${o.currency === "cny" ? 72_000 : 0}, ${o.start}, ${o.increment},
        ${o.reserve}, ${3 * DAY},
        ${sql.json({ instant: false, ship_window: 3 * DAY, confirm_window: 7 * DAY, fee_payer: 0 })},
        sha256(${`e2e-auction-content-${topicId}`}::bytea), '{}', sha256(${`e2e-auction-${topicId}`}::bytea),
        '\\x01', 1, ${ends}::timestamptz - interval '3 days', ${ends}, ${ends}::timestamptz + interval '1 day',
        900, 900, 86400
      FROM topics t JOIN wallets w ON w.user_id = ${seller.id} AND w.chain_id = ${chain}
      WHERE t.id = ${topicId}
      RETURNING id::int AS id`;
    if (!row) throw new Error("auction not created");
    return row.id;
  });
  return { topicId, auctionId, title };
}

/** 经同源 API 出价（另一位用户），返回状态码。 */
async function bid(m: Member, auctionId: number, amount: number): Promise<number> {
  const res = await m.page.request.post(`/api/v1/auctions/${auctionId}/bids`, {
    headers: { origin: origin() },
    data: { amount: String(amount) },
  });
  return res.status();
}

const panel = (page: Page) => page.getByRole("complementary", { name: "出价" });

/** 卖家与两位出价人（都已开通交易），并打开拍卖总开关。 */
async function cast(browser: Browser, base: string, tag: string) {
  const people = await Promise.all(
    ["s", "a", "b"].map((r) => member(browser, base, `${tag}${r}`, { username: true })),
  );
  const [seller, a, b] = people as [Member, Member, Member];
  const chain = await chainId(seller);
  await Promise.all(people.map((m) => trader(m, chain)));
  await enableAuctions();
  return { seller, a, b, chain, close: () => Promise.all(people.map((m) => m.context.close())) };
}

test("bid flow: next minimum, CNY → USDT confirm, own bid, outbid push from another bidder", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const { seller, a, b, chain, close } = await cast(
    browser,
    baseURL ?? "",
    `ab${info.project.name}`,
  );
  try {
    // ¥500 起拍、幅度 ¥20；另一位先出 ¥1,280
    const s = await liveAuction(seller, chain, {
      currency: "cny",
      start: 50_000,
      increment: 2_000,
      reserve: 100_000,
      endsInMs: 2 * HOUR,
    });
    expect(await bid(b, s.auctionId, 128_000)).toBe(201);

    const page = a.page;
    await page.goto(`/t/${s.topicId}`);
    const p = panel(page);
    await expect(p.getByTestId("auction-price")).toHaveText("¥1,280.00");
    await expect(p).toContainText("按卖家汇率 7.2000 折合 177.777778 USDT");
    await expect(p).toContainText(/1:59:\d\d/);
    await expect(p).toContainText("结束前 15 分钟内有新出价，自动顺延 15 分钟");
    // 下一口 = 当前价 + max(幅度 ¥20, 1 USDT, 当前价 1%)
    await expect(p).toContainText("最低出价 ¥1,300.00（折合 180.555556 USDT）");
    await expect(p.getByRole("list", { name: "出价记录" }).getByRole("listitem")).toHaveCount(1);
    await expect(p).toContainText("已达保留价");
    await expect(page.getByRole("complementary", { name: "出价" })).toContainText("不限出价资格");

    const input = p.getByLabel("你的出价（人民币）");
    await input.fill("1299.99");
    await expect(p.getByRole("alert")).toHaveText("至少出 ¥1,300.00");
    await p.getByRole("button", { name: "出价 ¥1,320.00" }).click();
    await expect(input).toHaveValue("1320.00");
    await p.getByRole("button", { name: "出价", exact: true }).click();
    const confirm = p.getByRole("group", { name: "确认出价" });
    await expect(confirm).toContainText("确认出价 ¥1,320.00？");
    await expect(confirm.getByTestId("bid-usdt")).toContainText("成交时货款 183.333334 USDT");
    await confirm.getByRole("button", { name: "确认出价" }).click();
    await expect(p.getByRole("status").filter({ hasText: "出价成功" })).toBeVisible();
    await expect(p.getByTestId("auction-price")).toHaveText("¥1,320.00");
    await expect(p.getByRole("listitem").first()).toContainText("（你）");

    // 别人超价：主题频道的推送更新价格、次数与记录，并提醒被超价
    expect(await bid(b, s.auctionId, 134_000)).toBe(201);
    await expect(page.getByRole("alert").filter({ hasText: "你的出价已被超过" })).toBeVisible();
    await expect(p.getByTestId("auction-price")).toHaveText("¥1,340.00");
    await expect(p.getByTestId("bid-count")).toHaveText("3 次");
    await expect(p.getByRole("status").filter({ hasText: "出价成功" })).toHaveCount(0);
    await expect(p.getByRole("listitem").first()).not.toContainText("（你）");
    await expect(p).toContainText("最低出价 ¥1,360.00");

    // 卖家本人：没有出价框
    await seller.page.goto(`/t/${s.topicId}`);
    await expect(panel(seller.page)).toContainText("卖家本人不能出价");
  } finally {
    await close();
  }
});

test("anti-snipe: inside the window the clock warns; a late bid pushes the end time out", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const base = baseURL ?? "";
  const { seller, a, b, chain, close } = await cast(browser, base, `as${info.project.name}`);
  const guest = await browser.newContext({ baseURL: base });
  try {
    const s = await liveAuction(seller, chain, {
      currency: "usdt",
      start: 20_000_000,
      increment: 1_000_000,
      reserve: 20_000_000,
      endsInMs: 10 * 60_000,
    });

    // 游客：起拍价、进入防狙击窗口的倒计时换成警示色，出价框换成登录链接
    const gp = await guest.newPage();
    await gp.goto(`/t/${s.topicId}`);
    const g = panel(gp);
    await expect(g.getByTestId("auction-price")).toHaveText("20.00 USDT");
    await expect(g).toContainText("起拍价");
    await expect(g).not.toContainText("折合");
    await expect(g.locator(".text-warning").filter({ hasText: /^(09|10):\d\d$/ })).toBeVisible();
    await expect(g.getByRole("link", { name: "登录后出价" })).toHaveAttribute(
      "href",
      /\/login\?next=/,
    );

    const page = a.page;
    await page.goto(`/t/${s.topicId}`);
    const p = panel(page);
    await expect(p.getByLabel("你的出价（USDT）")).toBeVisible();
    // 窗口内的出价顺延 15 分钟：推送带来新的结束时间
    expect(await bid(b, s.auctionId, 20_000_000)).toBe(201);
    await expect(p.getByRole("status").filter({ hasText: "结束时间顺延至" })).toBeVisible();
    await expect(p).toContainText("已顺延 15 分钟");
    await expect(p).toContainText(/2[45]:\d\d/);
    await expect(p.getByTestId("auction-price")).toHaveText("20.00 USDT");
    await expect(p).toContainText("最低出价 21.00 USDT");
  } finally {
    await guest.close();
    await close();
  }
});

test("after the end: the winner gets 去付款, others see the deadline; runner-up; unsold", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(90_000);
  const { seller, a, b, chain, close } = await cast(
    browser,
    baseURL ?? "",
    `ae${info.project.name}`,
  );
  try {
    const s = await liveAuction(seller, chain, {
      currency: "cny",
      start: 50_000,
      increment: 2_000,
      reserve: 50_000,
      endsInMs: 2 * HOUR,
    });
    expect(await bid(b, s.auctionId, 50_000)).toBe(201);
    expect(await bid(a, s.auctionId, 128_000)).toBe(201);
    await a.page.goto(`/t/${s.topicId}`);
    await expect(panel(a.page).getByLabel("你的出价（人民币）")).toBeVisible();

    // 结束并签发第 round 份成交证明（forum-worker 经 forum-sponsor 做的事，这里直接写库）
    const offer = (round: number, to: Member) =>
      withDb(
        (sql) => sql`WITH p AS (
            INSERT INTO auction_proofs (auction_id, round, bid_id, bidder_id, auth_hash, winner, price, pay_deadline,
                                        signature)
            SELECT a.id, ${round}, b.id, b.bidder_id, a.auth_hash, w.address, b.amount,
                   now() + interval '47 hours', '\\x01'
            FROM auctions a
            JOIN auction_bids b ON b.auction_id = a.id AND b.bidder_id = ${to.id}
            JOIN wallets w ON w.user_id = b.bidder_id AND w.chain_id = a.chain_id
            WHERE a.id = ${s.auctionId}
            ORDER BY b.amount DESC LIMIT 1
            RETURNING pay_deadline)
          UPDATE auctions SET state = 2, round = ${round}, pay_deadline = p.pay_deadline,
                 ends_at = now() - interval '1 minute'
          FROM p WHERE id = ${s.auctionId}`,
      );
    await offer(1, a);
    await a.page.reload();
    const p = panel(a.page);
    await expect(p).toContainText("成交待付款");
    await expect(p).toContainText("你赢得了本场拍卖");
    await expect(p).toContainText("付款截止还剩");
    await expect(p.getByRole("link", { name: "去付款" })).toHaveAttribute(
      "href",
      new RegExp(`/auction/pay\\?auction=${s.auctionId}$`),
    );
    await expect(p.getByLabel("你的出价（人民币）")).toHaveCount(0);

    // 其他人：看到付款截止，没有付款按钮
    await b.page.goto(`/t/${s.topicId}`);
    const q = panel(b.page);
    await expect(q).toContainText("平台已签发成交证明，等待赢家付款");
    await expect(q.getByRole("link", { name: "去付款" })).toHaveCount(0);

    // 赢家逾期：第 2 份证明给次高者 b
    await offer(2, b);
    await b.page.reload();
    await expect(q).toContainText("原赢家逾期未付款，你可以按自己的出价递补成交");
    await expect(q.getByRole("link", { name: "去付款" })).toBeVisible();

    // 递补也用完：流拍
    await withDb((sql) => sql`UPDATE auctions SET state = 4 WHERE id = ${s.auctionId}`);
    await b.page.reload();
    await expect(q).toContainText("流拍");
    await expect(q).toContainText("最高出价");
    await expect(q.getByRole("link", { name: "去付款" })).toHaveCount(0);
  } finally {
    await close();
  }
});

test("board list: the auction row shows the current price and links to the auction topic", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(60_000);
  const { seller, b, chain, close } = await cast(browser, baseURL ?? "", `al${info.project.name}`);
  try {
    const s = await liveAuction(seller, chain, {
      currency: "cny",
      start: 50_000,
      increment: 2_000,
      reserve: 100_000,
      endsInMs: 2 * HOUR,
    });
    expect(await bid(b, s.auctionId, 128_000)).toBe(201);
    const page = b.page;
    await page.goto(`/f/${BOARD}`);
    const row = page.getByTestId("topic-row").filter({ hasText: s.title });
    await expect(row).toContainText("拍卖");
    await expect(row).toContainText("当前 ¥1,280.00");
    await expect(row).toContainText("1 次出价");
    await expect(row).toContainText("已达保留价");
    await expect(row.getByRole("link", { name: s.title })).toHaveAttribute(
      "href",
      `/t/${s.topicId}`,
    );
  } finally {
    await close();
  }
});

test("create: CNY terms with live USDT hints, validation, the draft, then hand-off to the wallet", async ({
  browser,
  baseURL,
}, info) => {
  test.setTimeout(60_000);
  const { seller, close } = await cast(browser, baseURL ?? "", `ac${info.project.name}`);
  try {
    const page = seller.page;
    await page.route(/\/auction\/sign\?/, (route) =>
      route.fulfill({ contentType: "text/html", body: "<p>wallet</p>" }),
    );
    await page.goto(`/f/${BOARD}`);
    await page.getByRole("link", { name: "发拍卖帖" }).click();
    await expect(page).toHaveURL(new RegExp(`/f/${BOARD}/auction$`));

    const form = page.getByRole("form", { name: "发起拍卖" });
    await form.getByLabel("标题", { exact: true }).fill("绝版手办");
    await page.getByRole("textbox", { name: "正文" }).fill("未拆封，带原盒");
    await form.getByRole("radio", { name: /^人民币/ }).check();
    await form.getByLabel("我的汇率（1 USDT 合多少元）").fill("72");
    await form.getByLabel("起拍价（人民币）").fill("500");
    await form.getByLabel("加价幅度（人民币）").fill("5");
    await form.getByRole("button", { name: "发布并去钱包签名" }).click();
    // 汇率填错（72 这类）时先报汇率；按 USDT 折算的金额校验要等汇率对了才做
    await expect(form).toContainText("汇率需在 5.0000–10.0000 之间");
    await expect(form).not.toContainText("加价幅度不能低于");

    await form.getByLabel("我的汇率（1 USDT 合多少元）").fill("7.2");
    // 起拍价折合与合约换算一致（¥500 ÷ 7.2 向上取整）
    await expect(form).toContainText("折合 69.444445 USDT");
    await expect(form).toContainText("至少 ¥7.20");
    await form.getByRole("button", { name: "发布并去钱包签名" }).click();
    await expect(form).toContainText("加价幅度不能低于 1 USDT 或起拍价的 1%");
    await form.getByLabel("加价幅度（人民币）").fill("10");
    await form.getByLabel("保留价（人民币，选填）").fill("800");
    await form.getByLabel("出价资格").selectOption("50");
    await form.getByRole("button", { name: "发布并去钱包签名" }).click();

    // 后端保存草稿（还不发帖），钱包按拍卖编号自己取授权、核对后再签
    await expect(page).toHaveURL(/\/auction\/sign\?auction=\d+$/);
    const id = Number(new URL(page.url()).searchParams.get("auction"));
    const res = await page.request.get(`/api/v1/auctions/${id}`);
    expect(res.status(), await res.text()).toBe(200);
    expect(await res.json()).toMatchObject({
      state: "draft",
      topic_id: null,
      currency: "cny",
      cny_per_usdt: 72_000,
      start_price: "50000",
      increment: "1000",
      reserve_price: "80000",
      duration_secs: 3 * DAY,
      eligibility: { kind: "percent", percent: 50 },
      terms: { instant: false, ship_window: 3 * DAY, confirm_window: 7 * DAY, fee_payer: 0 },
    });
  } finally {
    await close();
  }
});

test("design mockup renders the same panel with mock data and previews the states", async ({
  page,
}) => {
  await page.goto("/design/v2/auction");
  const p = panel(page);
  await expect(p.getByTestId("auction-price")).toHaveText("¥1,280.00");
  await p.getByRole("radio", { name: "成交待付款" }).click();
  await expect(p).toContainText("你赢得了本场拍卖");
  await p.getByRole("radio", { name: "流拍" }).click();
  await expect(p).toContainText("未达保留价，本场流拍");
});
