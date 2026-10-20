import { Button, Card, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import { WalletNav } from "../components/WalletNav";

export const metadata: Metadata = { title: "页面不存在" };

export default function NotFound() {
  return (
    <>
      <WalletNav />
      <main className="mx-auto max-w-xl px-4 py-5">
        <Card>
          <EmptyState
            title="钱包里没有这个页面"
            description="链接可能有误。付款、报价请从论坛的交易帖或私信进入。"
            action={
              <Button asChild variant="secondary" size="sm">
                <a href="/">回钱包首页</a>
              </Button>
            }
          />
        </Card>
      </main>
    </>
  );
}
