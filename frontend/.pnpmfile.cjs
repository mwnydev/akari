// typescript-eslint 8.x 尚不支持 TS 7 的原生 API（见 TS 7.0 发布说明「与 TS 6 并行」）。
// 类型检查用 typescript 7.0.2 的 tsc；ESLint 相关包改为依赖兼容包 @typescript/typescript6。
// typescript-eslint 支持 TS 7 后删除本文件。
const TS6 = "npm:@typescript/typescript6@6.0.2";
const USES_TS_API = /^(typescript-eslint|@typescript-eslint\/.+|ts-api-utils)$/;

function readPackage(pkg) {
  if (USES_TS_API.test(pkg.name) && pkg.peerDependencies && pkg.peerDependencies.typescript) {
    delete pkg.peerDependencies.typescript;
    pkg.dependencies = { ...pkg.dependencies, typescript: TS6 };
  }
  return pkg;
}

module.exports = { hooks: { readPackage } };
