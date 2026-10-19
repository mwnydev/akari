import type { Metadata } from "next";
import { EmergencyWithdraw } from "../../components/emergency/EmergencyWithdraw";

export const metadata: Metadata = { title: "紧急取回" };

/**
 * 紧急取回（ADR 0007）：论坛停摆时也能用。不带 WalletNav（它链到依赖后端的页面），
 * 本路由的代码不引用 api-client（eslint 规则把关），读链和提交只连用户选的公共节点。
 */
export default function EmergencyPage() {
  return (
    <main className="mx-auto max-w-3xl px-4 py-5">
      <EmergencyWithdraw />
    </main>
  );
}
