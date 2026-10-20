import { Button } from "@akari/ui";
import { MessageCircle, ShieldCheck } from "lucide-react";
import Link from "next/link";
import { loginHref } from "../../lib/links";
import { WALLET_ORIGIN } from "../../lib/public-config";
import { StartConversation } from "../messages/StartConversation";
import { WalletLink } from "../settings/UsernameGate";

/**
 * 面板里的操作。买家：「担保购买」去钱包付卖家的公开报价；「私信卖家」「发起担保」先打开与卖家的私信会话，
 * 后者再去钱包在会话里提出自定义报价（与私信页同一个入口）。卖家本人：去钱包发布公开报价。
 */
export function TradeActions({
  listingId: id,
  sellerId,
  viewerId,
  offerId,
  here,
}: {
  listingId: number;
  sellerId: number;
  /** 登录用户；游客为 null */
  viewerId: number | null;
  /** 可购买的公开报价 */
  offerId: number | undefined;
  /** 游客登录后回到的地址 */
  here: string;
}) {
  if (viewerId === sellerId) {
    return (
      <>
        <Button asChild size="lg" className="w-full">
          <WalletLink href={`${WALLET_ORIGIN}/offer?listing=${id}`}>
            <ShieldCheck size={18} aria-hidden />
            发布公开报价
          </WalletLink>
        </Button>
        <p className="text-xs text-fg-muted">
          这是你的交易帖：在钱包里签名公开报价后，买家就能直接担保购买。
        </p>
      </>
    );
  }
  return (
    <>
      {offerId !== undefined ? (
        <Button asChild size="lg" className="w-full">
          <WalletLink href={`${WALLET_ORIGIN}/pay?offer=${offerId}`}>
            <ShieldCheck size={18} aria-hidden />
            担保购买
          </WalletLink>
        </Button>
      ) : (
        <>
          <Button size="lg" className="w-full" disabled>
            <ShieldCheck size={18} aria-hidden />
            担保购买
          </Button>
          <p className="text-xs text-fg-muted">
            卖家还没有发布报价：成交以卖家在钱包里签名的报价为准，也可以私信卖家发起担保。
          </p>
        </>
      )}
      <div className="grid grid-cols-2 gap-2">
        {viewerId !== null ? (
          <>
            <StartConversation
              userId={sellerId}
              label="私信卖家"
              variant="secondary"
              size="md"
              className="w-full"
            />
            <StartConversation
              userId={sellerId}
              then="offer"
              variant="secondary"
              size="md"
              className="w-full"
            />
          </>
        ) : (
          <>
            <Button asChild variant="secondary">
              <Link href={loginHref(here)}>
                <MessageCircle size={16} aria-hidden />
                私信卖家
              </Link>
            </Button>
            <Button asChild variant="secondary">
              <Link href={loginHref(here)}>
                <ShieldCheck size={16} aria-hidden />
                发起担保
              </Link>
            </Button>
          </>
        )}
      </div>
      <p className="text-xs text-fg-muted">
        「发起担保」：在私信里提出自定义数量、价格或保证金，卖家同意后签名报价。
      </p>
    </>
  );
}
