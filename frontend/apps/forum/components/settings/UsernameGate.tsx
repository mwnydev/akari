"use client";

import { usePathname } from "next/navigation";
import {
  Suspense,
  createContext,
  lazy,
  useCallback,
  useContext,
  useMemo,
  useState,
  useSyncExternalStore,
  type MouseEvent,
  type ReactNode,
} from "react";
import { dismissPrompt, promptDismissed } from "../../lib/username-prompt";

// 弹窗与表单按需加载（只在要弹出时下载），不进每个页面的首屏 JS
const UsernameDialog = lazy(() => import("./UsernameDialog"));

interface Gate {
  /** 登录了但还没设置用户名 */
  needed: boolean;
  /** 先弹出设置用户名，保存后执行 `then`（不需要时直接执行） */
  ask: (then: () => void) => void;
}

const Ctx = createContext<Gate>({ needed: false, ask: (then) => then() });

const noSubscribe = () => () => undefined;

/**
 * 新用户设置用户名的提示（AppShell 里，只给登录了但没有用户名的人）：
 * - 第一次登录后在任意页面弹一次（「以后再说」后本机记住，不再主动弹；账号设置页本身不弹）；
 * - 交易入口（担保购买、报价、发起担保、开通交易）点击时每次都再问，保存后继续跳转。
 */
export function UsernameGate({ userId, children }: { userId: number | null; children: ReactNode }) {
  const pathname = usePathname();
  const [saved, setSaved] = useState(false);
  const [closed, setClosed] = useState(false);
  const [pending, setPending] = useState<{ then: () => void } | null>(null);
  // 服务端渲染当作「已记住」：只在浏览器里决定要不要弹，避免水合不一致
  const dismissed = useSyncExternalStore(
    noSubscribe,
    () => userId === null || promptDismissed(userId),
    () => true,
  );
  const needed = userId !== null && !saved;
  const welcome = needed && !dismissed && !closed && pathname !== "/settings";

  const ask = useCallback((then: () => void) => (needed ? setPending({ then }) : then()), [needed]);
  const gate = useMemo(() => ({ needed, ask }), [needed, ask]);

  const close = () => {
    if (userId !== null && !pending) dismissPrompt(userId);
    setClosed(true);
    setPending(null);
  };
  const done = () => {
    setSaved(true);
    const then = pending?.then;
    setPending(null);
    then?.();
  };

  return (
    <Ctx.Provider value={gate}>
      {children}
      {userId !== null && needed && (pending || welcome) ? (
        <Suspense fallback={null}>
          <UsernameDialog
            userId={userId}
            reason={pending ? "trade" : "welcome"}
            onSaved={done}
            onClose={close}
          />
        </Suspense>
      ) : null}
    </Ctx.Provider>
  );
}

export function useUsernameGate(): Gate {
  return useContext(Ctx);
}

/**
 * 去钱包的交易入口链接：没设置用户名时拦下点击，先弹出设置用户名，保存后再跳。
 * 可以放在 `<Button asChild>` 里（Slot 会把样式与事件合并到这个 `<a>` 上）。
 */
export function WalletLink({
  href,
  onClick,
  ...rest
}: React.AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  const gate = useUsernameGate();
  return (
    <a
      href={href}
      {...rest}
      onClick={(e: MouseEvent<HTMLAnchorElement>) => {
        onClick?.(e);
        if (e.defaultPrevented || !gate.needed) return;
        e.preventDefault();
        gate.ask(() => location.assign(href));
      }}
    />
  );
}
