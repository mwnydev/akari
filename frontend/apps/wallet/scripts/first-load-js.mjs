// 统计静态导出每个页面的首次加载 JS（HTML 里直接引用的 <script src>，gzip 后），对照 plan 第 12 节预算 200 KB。
// 动态 import 的分包（钱包 SDK）不在 HTML 里，不计入首屏；noModule 的旧浏览器补丁现代浏览器不下载，也不计。
// 用法：pnpm build && pnpm size
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const BUDGET = 200 * 1024;
const out = new URL("../out/", import.meta.url).pathname;
const kb = (n) => `${(n / 1024).toFixed(1)} KB`;

const chunkSize = new Map();
const gz = (src) => {
  if (!chunkSize.has(src)) chunkSize.set(src, gzipSync(readFileSync(join(out, src))).length);
  return chunkSize.get(src);
};

const pages = readdirSync(out, { recursive: true })
  .filter((f) => f.endsWith(".html") && !f.startsWith("_") && f !== "404.html")
  .sort();
let failed = false;
for (const page of pages) {
  const html = readFileSync(join(out, page), "utf8");
  const scripts = [
    ...new Set(
      [...html.matchAll(/<script([^>]+)>/g)]
        .map((m) => m[1] ?? "")
        .filter((attrs) => !/\bnoModule\b/i.test(attrs))
        .map((attrs) => /src="\/([^"]+\.js)"/.exec(attrs)?.[1])
        .filter(Boolean),
    ),
  ];
  const total = scripts.reduce((sum, s) => sum + gz(s), 0);
  if (total > BUDGET) failed = true;
  console.log(
    `${total > BUDGET ? "✗" : "✓"} ${kb(total).padStart(9)}  /${page.replace(/(index)?\.html$/, "")}`,
  );
}
const all = readdirSync(join(out, "_next/static/chunks")).filter((f) => f.endsWith(".js"));
const largest = all.map((f) => [f, gz(`_next/static/chunks/${f}`)]).sort((a, b) => b[1] - a[1])[0];
if (largest) console.log(`最大的分包：${largest[0]} ${kb(largest[1])}（gzip）`);
if (failed) process.exit(1);
