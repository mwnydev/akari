import { ApiError } from "@akari/api-client";
import { Button, Card, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { AuctionFormLoader } from "../../../../components/auction/AuctionFormLoader";
import { PostCaptchaProvider } from "../../../../components/captcha/PostCaptcha";
import { PageError } from "../../../../components/PageError";
import { AppShell } from "../../../../components/forum/AppShell";
import { load, serverApi } from "../../../../lib/api";
import { listForums } from "../../../../lib/forums";
import { boardHref, loginHref } from "../../../../lib/links";
import { needsPostCaptcha, WALLET_ORIGIN } from "../../../../lib/public-config";
import { getMe } from "../../../../lib/session";
import { WalletLink } from "../../../../components/settings/UsernameGate";

export const metadata: Metadata = { title: "发起拍卖", robots: { index: false } };

export default async function NewAuctionPage({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const here = `${boardHref(slug)}/auction`;
  const me = await getMe();
  if (!me) redirect(loginHref(here));

  const api = await serverApi();
  const [forums, wallet, tradeParams, auctionParams] = await Promise.all([
    listForums(),
    load(api.GET("/api/v1/wallet")),
    load(api.GET("/api/v1/trade/params")),
    load(api.GET("/api/v1/auctions/params")),
  ]);
  if (
    forums instanceof ApiError ||
    wallet instanceof ApiError ||
    tradeParams instanceof ApiError ||
    auctionParams instanceof ApiError
  ) {
    return (
      <AppShell active={slug}>
        <PageError
          error={[forums, wallet, tradeParams, auctionParams].find((r) => r instanceof ApiError)}
        />
      </AppShell>
    );
  }
  const forum = forums.find((f) => f.slug === slug);
  // 拍卖帖只能发在拍卖板块（allow_auction），且拍卖总开关打开
  if (!forum || auctionParams.settings.enabled !== true || !forum.allow_auction) notFound();

  return (
    <AppShell active={slug}>
      <div className="max-w-3xl space-y-4">
        <nav aria-label="位置" className="text-sm text-fg-muted">
          <Link href={boardHref(slug)} className="hover:text-fg">
            {forum.name}
          </Link>{" "}
          / 发起拍卖
        </nav>
        <h1 className="text-2xl font-semibold">发起拍卖</h1>
        <Card>
          {wallet.enabled ? (
            <PostCaptchaProvider required={needsPostCaptcha(me)}>
              <AuctionFormLoader forumSlug={slug} trade={tradeParams} auction={auctionParams} />
            </PostCaptchaProvider>
          ) : (
            <EmptyState
              title="发拍卖帖需要先开通交易"
              description="开通交易会创建你的 Passkey 钱包，用于签名拍卖授权和收款；只需一次。"
              action={
                <Button asChild>
                  <WalletLink href={`${WALLET_ORIGIN}/setup`}>开通交易</WalletLink>
                </Button>
              }
            />
          )}
        </Card>
      </div>
    </AppShell>
  );
}
