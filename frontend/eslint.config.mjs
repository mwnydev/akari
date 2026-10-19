import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["**/.next/**", "**/out/**", "**/dist/**", "**/node_modules/**", "**/next-env.d.ts"] },
  js.configs.recommended,
  ...tseslint.configs.strict,
  {
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
    },
  },
  {
    // 紧急取回页（ADR 0007）必须在论坛后端停摆时照常可用：不能引用 api-client 和依赖它的钱包模块
    files: [
      "apps/wallet/app/emergency/**",
      "apps/wallet/components/emergency/**",
      "apps/wallet/lib/emergency/**",
    ],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: [
                "@akari/api-client",
                "**/wallet/api",
                "**/wallet/sdk",
                "**/wallet/load",
                "**/wallet/useWallet",
                "**/wallet/passkey",
                "**/wallet/sponsor",
                "**/WalletGate",
                "**/WalletNav",
              ],
              message: "紧急取回页不能依赖论坛后端（ADR 0007）",
            },
          ],
        },
      ],
    },
  },
  {
    // 构建辅助脚本（纯 Node ESM）
    files: ["**/scripts/**/*.mjs"],
    languageOptions: { globals: { console: "readonly", process: "readonly", URL: "readonly" } },
  },
);
