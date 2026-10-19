import Link from "next/link";
import { loginHref } from "../../lib/links";

/** 不能回复时回复栏里的说明：游客给登录链接，登录用户说明原因。 */
export function ReplyNotice({
  signedIn,
  locked,
  here,
}: {
  signedIn: boolean;
  locked: boolean;
  /** 登录后回到的地址 */
  here: string;
}) {
  return (
    <p className="min-w-0 flex-1 py-1.5 text-sm text-fg-muted">
      {!signedIn ? (
        <>
          <Link href={loginHref(here)} className="text-accent hover:underline">
            登录
          </Link>{" "}
          后参与回复
        </>
      ) : locked ? (
        "主题已锁定，不能回复"
      ) : (
        "你的等级暂时不能在本板块回复"
      )}
    </p>
  );
}
