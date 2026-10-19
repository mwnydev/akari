// 统计每个路由的首次加载 JS（gzip 后），对照 plan 第 12 节预算 200 KB。
// 论坛是服务端渲染，没有静态 HTML 可读：按构建清单取「所有页面共用的入口」+ 该路由的布局、出错页与页面入口，
// 与页面 HTML 里的 <script src> 一致。动态 import 的分包（编辑器、弹窗、私信面板等）不计入；noModule 补丁也不计。
// 用法：pnpm build && pnpm size
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import vm from "node:vm";

const BUDGET = 200 * 1024;
const next = new URL("../.next/", import.meta.url).pathname;
const kb = (n) => `${(n / 1024).toFixed(1)} KB`;

const chunkSize = new Map();
const gz = (file) => {
  if (!chunkSize.has(file)) {
    chunkSize.set(file, gzipSync(readFileSync(join(next, file.replace(/^\/_next\//, "")))).length);
  }
  return chunkSize.get(file);
};

const build = JSON.parse(readFileSync(join(next, "build-manifest.json"), "utf8"));
const appDir = join(next, "server/app");
const manifests = readdirSync(appDir, { recursive: true })
  .filter((f) => f.endsWith("page_client-reference-manifest.js"))
  .sort();

let failed = false;
for (const file of manifests) {
  const sandbox = { globalThis: {} };
  vm.runInNewContext(readFileSync(join(appDir, file), "utf8"), sandbox);
  for (const [route, manifest] of Object.entries(sandbox.globalThis.__RSC_MANIFEST)) {
    if (route.startsWith("/_")) continue;
    const scripts = new Set(build.rootMainFiles);
    for (const files of Object.values(manifest.entryJSFiles)) files.forEach((f) => scripts.add(f));
    const total = [...scripts].reduce((sum, s) => sum + gz(s), 0);
    if (total > BUDGET) failed = true;
    const path = route.replace(/\/page$/, "") || "/";
    console.log(`${total > BUDGET ? "✗" : "✓"} ${kb(total).padStart(9)}  ${path}`);
  }
}
if (failed) process.exit(1);
