// TS 7 是原生实现，没有 JS 编译器 API（见 TS 7.0 发布说明「与 TS 6 并行」）。
// 类型检查用 typescript 7.0.2 的 tsc；需要调用编译器 API 的工具（typescript-eslint、openapi-typescript）
// 改为依赖兼容包 @typescript/typescript6。这些工具支持 TS 7 后删除对应条目，全部支持后删除本文件。
const TS6 = "npm:@typescript/typescript6@6.0.2";
const USES_TS_API = /^(typescript-eslint|@typescript-eslint\/.+|ts-api-utils|openapi-typescript)$/;

function readPackage(pkg) {
  if (!USES_TS_API.test(pkg.name)) return pkg;
  const wantsTs =
    (pkg.peerDependencies && pkg.peerDependencies.typescript) ||
    (pkg.dependencies && pkg.dependencies.typescript);
  if (wantsTs) {
    if (pkg.peerDependencies) delete pkg.peerDependencies.typescript;
    pkg.dependencies = { ...pkg.dependencies, typescript: TS6 };
  }
  return pkg;
}

module.exports = { hooks: { readPackage } };
