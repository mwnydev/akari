// 构建后检查：DEV 代付私钥绝不能出现在静态导出产物里（lib/wallet/dev-sponsor.ts）。
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const source = readFileSync(new URL("../lib/wallet/dev-sponsor.ts", import.meta.url), "utf8");
const key = /0x[0-9a-f]{64}/.exec(source)?.[0];
if (!key) throw new Error("dev-sponsor.ts 里找不到开发私钥常量，检查脚本需要同步更新");
const needle = key.slice(2);

const outDir = new URL("../out/", import.meta.url).pathname;
const leaks = readdirSync(outDir, { recursive: true, withFileTypes: true })
  .filter((d) => d.isFile())
  .map((d) => join(d.parentPath, d.name))
  .filter((f) => readFileSync(f, "latin1").includes(needle));
if (leaks.length > 0) {
  console.error(`DEV 代付私钥出现在构建产物里：\n${leaks.join("\n")}`);
  process.exit(1);
}
console.log("✓ 构建产物不含 DEV 代付私钥");
