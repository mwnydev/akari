// 论坛压测（plan 第 12 节）：3000 在线、峰值 1200 请求/秒的流量组合，另做一次 2 倍压力确认余量。
// 由 deploy/loadtest/run.sh 用 grafana/k6 镜像运行；目标数据（targets.json、sessions.txt）挂载在 /data。
//
// 环境变量：BASE（forum-api 地址）、ORIGIN（ALLOWED_ORIGINS 里的论坛来源）、SCALE（流量倍数，默认 1）、
// DURATION（默认 5m）、WS_CONNS（WebSocket 长连接数，默认 300，0 为不测；每条连接是一个 VU，约占 k6 5–7 MB）、WS_STORM（设置时所有连接同时建立）、OUT（汇总 JSON 路径）。
import http from "k6/http";
import ws from "k6/ws";
import { check, sleep } from "k6";
import { Counter, Trend } from "k6/metrics";

const BASE = __ENV.BASE || "http://127.0.0.1:8480";
const ORIGIN = __ENV.ORIGIN || "http://localhost:3000";
const SCALE = Number(__ENV.SCALE || "1");
const DURATION = __ENV.DURATION || "5m";
const WS_CONNS = Number(__ENV.WS_CONNS || "300");

const T = JSON.parse(open("/data/targets.json"));
const SESSIONS = open("/data/sessions.txt").split("\n").filter(Boolean);

// 搜索词：常见词（命中几十万楼）、两个词同时出现、少见短语、词中间的字母、没有结果的词
const SEARCH = [
  "出售", "账号", "USDT", "担保", "Netflix", "会员", "教程", "节点",
  "出售 账号", "担保 交易", "港版 未拆封", "自动发货 月卡", "香港 线路",
  "回程 晚高峰", "苹果ID 兑换码", "PostgreSQL 索引", "显卡 二手 保修",
  "gpt", "flix", "量子纠缠", "某个不存在的词组",
];

const rate = (perSec) => Math.max(1, Math.round(perSec * SCALE));
const vus = (perSec) => Math.max(10, Math.round(perSec * SCALE * 0.2));

function arrival(fn, perSec) {
  return {
    executor: "constant-arrival-rate",
    exec: fn,
    rate: rate(perSec),
    timeUnit: "1s",
    duration: DURATION,
    preAllocatedVUs: vus(perSec),
    maxVUs: vus(perSec) * 10,
  };
}

export const options = {
  discardResponseBodies: true,
  scenarios: {
    guest_read: arrival("guestRead", 700),
    user_read: arrival("userRead", 380),
    reply: arrival("reply", 10),
    new_topic: arrival("newTopic", 1),
    like: arrival("like", 10),
    search: arrival("search", 25),
    ...(WS_CONNS > 0
      ? {
          websocket: {
            executor: "constant-vus",
            exec: "socket",
            vus: Math.round(WS_CONNS * SCALE),
            duration: DURATION,
          },
        }
      : {}),
  },
  // plan 第 12 节：帖子页、板块页 API p95 < 50 ms、p99 < 150 ms；发帖回复 p95 < 100 ms；搜索 p95 < 300 ms
  thresholds: {
    "http_req_failed{kind:http}": ["rate<0.01"],
    "http_req_duration{ep:topic}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:topic_deep}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:board}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:board_deep}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:board_cursor}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:latest}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:forums}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:me}": ["p(95)<50", "p(99)<150"],
    "http_req_duration{ep:reply}": ["p(95)<100"],
    "http_req_duration{ep:new_topic}": ["p(95)<100"],
    "http_req_duration{ep:like}": ["p(95)<100"],
    "http_req_duration{ep:search}": ["p(95)<300"],
    "http_req_duration{ep:search_topic}": ["p(95)<300"],
    ws_ready: ["p(95)<1000"],
    // 同样的浏览接口：游客走 Valkey 游客缓存，登录用户每次回源（两者对比即缓存的效果）
    "http_req_duration{who:guest}": ["p(95)<50"],
    "http_req_duration{who:user}": ["p(95)<50"],
  },
  summaryTrendStats: ["avg", "p(50)", "p(95)", "p(99)", "max", "count"],
};

const wsReady = new Trend("ws_ready", true);
const wsPushes = new Counter("ws_pushes");

/** 偏斜抽样：少数热门目标拿到大部分流量。 */
function skewed(list) {
  return list[Math.floor(list.length * Math.random() ** 3)];
}
function pick(list) {
  return list[Math.floor(Math.random() * list.length)];
}
/** 模拟大量不同客户端：forum-api 只信任来自本机（Caddy）的 CF-Connecting-IP。 */
function clientIp() {
  return `10.${(Math.random() * 255) | 0}.${(Math.random() * 255) | 0}.${1 + ((Math.random() * 254) | 0)}`;
}
function headers(cookie) {
  const h = { "CF-Connecting-IP": clientIp() };
  if (cookie) h.Cookie = cookie;
  return h;
}
function get(path, ep, cookie) {
  const who = ep.startsWith("search") ? "search" : cookie ? "user" : "guest";
  const res = http.get(`${BASE}${path}`, { headers: headers(cookie), tags: { ep, kind: "http", who, name: ep } });
  check(res, { [`${ep} 2xx`]: (r) => r.status >= 200 && r.status < 300 });
  return res;
}
function send(method, path, body, ep, cookie) {
  const res = http.request(method, `${BASE}${path}`, body ? JSON.stringify(body) : null, {
    headers: { ...headers(cookie), Origin: ORIGIN, "Content-Type": "application/json" },
    tags: { ep, kind: "http", name: ep },
  });
  check(res, { [`${ep} 2xx`]: (r) => r.status >= 200 && r.status < 300 });
  return res;
}
const session = () => pick(SESSIONS);
const listedBoards = T.boards.filter((b) => b.pages > 0);

/** 一次页面浏览对应的接口（与 SSR 渲染页面时调用的接口一致）。 */
function browse(cookie) {
  const x = Math.random();
  if (x < 0.45) {
    const [id] = skewed(T.topics);
    get(`/api/v1/topics/${id}`, "topic", cookie);
  } else if (x < 0.6) {
    // 热帖的随机页（含最后一页）与普通帖的后几页
    const [id, floors] = Math.random() < 0.5 ? pick(T.hot) : skewed(T.topics);
    const pages = Math.max(1, Math.ceil(floors / 20));
    const page = Math.random() < 0.3 ? pages : 1 + Math.floor(Math.random() * pages);
    get(`/api/v1/topics/${id}?page=${page}`, "topic_deep", cookie);
  } else if (x < 0.75) {
    get(`/api/v1/forums/${skewed(listedBoards).slug}/topics`, "board", cookie);
  } else if (x < 0.87) {
    const b = skewed(listedBoards);
    const page = 2 + Math.floor(Math.random() ** 2 * Math.max(1, b.pages - 1));
    get(`/api/v1/forums/${b.slug}/topics?page=${Math.min(page, b.pages)}`, "board_deep", cookie);
  } else if (x < 0.9 && T.deep.length) {
    const d = pick(T.deep);
    get(`/api/v1/forums/${d.slug}/topics?cursor=${encodeURIComponent(d.cursor)}`, "board_cursor", cookie);
  } else if (x < 0.98) {
    get("/api/v1/topics/latest", "latest", cookie);
  } else {
    get("/api/v1/forums", "forums", cookie);
  }
}

export function guestRead() {
  browse(null);
}

export function userRead() {
  const cookie = session();
  if (Math.random() < 0.1) {
    get("/api/v1/me", "me", cookie);
  } else {
    browse(cookie);
  }
}

const doc = (text) => ({
  type: "doc",
  content: [{ type: "paragraph", content: [{ type: "text", text }] }],
});
const words = "压测 回复 担保 交易 出售 自动发货 好评 谢谢楼主 支持 顶一下 已收到 发货很快".split(" ");
const sentence = () =>
  Array.from({ length: 6 + ((Math.random() * 20) | 0) }, () => pick(words)).join("");

export function reply() {
  const text = sentence();
  send(
    "POST",
    `/api/v1/topics/${pick(T.writable)}/posts`,
    { html: `<p>${text}</p>`, source: doc(text) },
    "reply",
    session(),
  );
}

export function newTopic() {
  const board = pick(T.boards.filter((b) => b.postable));
  const text = sentence();
  send(
    "POST",
    `/api/v1/forums/${board.slug}/topics`,
    { title: `压测主题 ${text.slice(0, 20)}`, html: `<p>${text}</p>`, source: doc(text) },
    "new_topic",
    session(),
  );
}

export function like() {
  const cookie = session();
  // 一半赞在热帖第 1 页上（WebSocket 连接订阅的是热帖，用来测推送）
  const post = Math.random() < 0.5 && T.hot_posts.length ? pick(T.hot_posts) : pick(T.posts);
  if (Math.random() < 0.7) send("PUT", `/api/v1/posts/${post}/like`, null, "like", cookie);
  else send("DELETE", `/api/v1/posts/${post}/like`, null, "like", cookie);
}

export function search() {
  const cookie = Math.random() < 0.4 ? session() : null;
  const q = encodeURIComponent(pick(SEARCH));
  if (Math.random() < 0.7) get(`/api/v1/search?q=${q}`, "search", cookie);
  else get(`/api/v1/search?q=${q}&type=topic`, "search_topic", cookie);
}

/** 长连接：连上、等 ready、订阅一个热帖（收它的点赞与评论推送），保持到场景结束。 */
export function socket() {
  // 第一次连接在前 30 秒里错开（所有连接同时建立是重启后的重连风暴，单独测：WS_STORM=1）
  if (__ITER === 0 && !__ENV.WS_STORM) sleep(Math.random() * 30);
  const url = `${BASE.replace(/^http/, "ws")}/api/v1/ws`;
  const started = Date.now();
  const res = ws.connect(
    url,
    { headers: { ...headers(session()), Origin: ORIGIN }, tags: { kind: "ws" } },
    (s) => {
      s.on("message", (raw) => {
        const msg = JSON.parse(raw);
        if (msg.type === "ready") {
          wsReady.add(Date.now() - started);
          const [topic] = T.hot.length ? pick(T.hot) : skewed(T.topics);
          s.send(JSON.stringify({ type: "subscribe", topic_id: topic }));
        } else if (msg.type === "likes" || msg.type === "comment") {
          wsPushes.add(1);
        }
      });
      s.setInterval(() => s.send(JSON.stringify({ type: "ping" })), 25000);
      s.setTimeout(() => s.close(), 60000 + Math.random() * 60000);
    },
  );
  check(res, { "ws 101": (r) => r && r.status === 101 });
}

export function handleSummary(data) {
  const out = __ENV.OUT || "/out/summary.json";
  const rows = Object.entries(data.metrics)
    .filter(([name]) => name.startsWith("http_req_duration{") || name === "ws_ready")
    .map(([name, m]) => {
      const v = m.values;
      const f = (x) => (x === undefined ? "-" : x.toFixed(1));
      return `${name.padEnd(36)} n=${String(v.count).padStart(7)} p50=${f(v["p(50)"])} p95=${f(v["p(95)"])} p99=${f(v["p(99)"])} max=${f(v.max)}`;
    });
  const reqs = data.metrics.http_reqs.values;
  const failed = data.metrics["http_req_failed{kind:http}"];
  rows.push(`http_reqs total=${reqs.count} rate=${reqs.rate.toFixed(1)}/s failed=${failed ? (failed.values.rate * 100).toFixed(2) : "-"}%`);
  rows.push(`ws_pushes=${data.metrics.ws_pushes ? data.metrics.ws_pushes.values.count : 0}`);
  const failedThresholds = Object.entries(data.metrics)
    .filter(([, m]) => m.thresholds && Object.values(m.thresholds).some((t) => !t.ok))
    .map(([name]) => name);
  rows.push(failedThresholds.length ? `THRESHOLDS FAILED: ${failedThresholds.join(", ")}` : "all thresholds passed");
  return { [out]: JSON.stringify(data, null, 2), stdout: rows.join("\n") + "\n" };
}
