import { ApiError } from "@akari/api-client";
import { Button, Card, EmptyState } from "@akari/ui";
import type { Metadata } from "next";
import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { PostCaptchaProvider } from "../../../../components/captcha/PostCaptcha";
import { PageError } from "../../../../components/PageError";
import { AppShell } from "../../../../components/forum/AppShell";
import { ListingFormLoader } from "../../../../components/trade/ListingFormLoader";
import { load, serverApi } from "../../../../lib/api";
import { listForums } from "../../../../lib/forums";
import { boardHref, loginHref } from "../../../../lib/links";
import { needsPostCaptcha, WALLET_ORIGIN } from "../../../../lib/public-config";
import { getMe } from "../../../../lib/session";
import { WalletLink } from "../../../../components/settings/UsernameGate";

export const metadata: Metadata = { title: "发起交易帖", robots: { index: false } };

export default async function NewListingPage({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const here = `${boardHref(slug)}/listing`;
  const me = await getMe();
  if (!me) redirect(loginHref(here));

  const api = await serverApi();
  const [forums, wallet, tradeParams] = await Promise.all([
    listForums(),
    load(api.GET("/api/v1/wallet")),
    load(api.GET("/api/v1/trade/params")),
  ]);
  if (forums instanceof ApiError || wallet instanceof ApiError || tradeParams instanceof ApiError) {
    return (
      <AppShell active={slug}>
        <PageError error={[forums, wallet, tradeParams].find((r) => r instanceof ApiError)} />
      </AppShell>
    );
  }
  const forum = forums.find((f) => f.slug === slug);
  if (!forum) notFound();

  return (
    <AppShell active={slug}>
      <div className="max-w-3xl space-y-4">
        <nav aria-label="位置" className="text-sm text-fg-muted">
          <Link href={boardHref(slug)} className="hover:text-fg">
            {forum.name}
          </Link>{" "}
          / 发起交易帖
        </nav>
        <h1 className="text-2xl font-semibold">发起交易帖</h1>
        <Card>
          {wallet.enabled ? (
            <PostCaptchaProvider required={needsPostCaptcha(me)}>
              <ListingFormLoader forumSlug={slug} params={tradeParams} />
            </PostCaptchaProvider>
          ) : (
            <EmptyState
              title="发交易帖需要先开通交易"
              description="开通交易会创建你的 Passkey 钱包，用于签名报价和收款；只需一次。"
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
