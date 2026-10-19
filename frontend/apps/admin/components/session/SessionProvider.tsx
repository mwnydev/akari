"use client";

import { ApiError } from "@akari/api-client";
import { Button, ErrorState, LoadingState } from "@akari/ui";
import { useQueryClient } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { api, onApiResult, unwrap, type AdminSessionView } from "../../lib/api";
import {
  clockFrom,
  reverifyFresh,
  sessionPhase,
  touch,
  type SessionClock,
} from "../../lib/session";
import { nowSecs, useNowSecs } from "../../lib/useNow";

// 登录页（表单库、WebAuthn）与再验证弹窗只在需要时加载
const LoginGate = dynamic(() => import("./LoginGate").then((m) => m.LoginGate), {
  ssr: false,
  loading: () => <LoadingState />,
});
const ReverifyDialog = dynamic(() => import("./ReverifyDialog").then((m) => m.ReverifyDialog), {
  ssr: false,
});

type State =
  | { kind: "loading" }
  | { kind: "error"; error: ApiError }
  | { kind: "anon"; expired: "idle" | "absolute" | "server" | null }
  | { kind: "in"; clock: SessionClock };

interface AdminSession {
  clock: SessionClock;
  /** 服务端返回了新的会话视图（登录、再验证、续期）。 */
  adopt: (view: AdminSessionView) => void;
  /** 顺延闲置期限（GET 一次会话）。 */
  keepAlive: () => Promise<void>;
  logout: () => Promise<void>;
  /**
   * 敏感操作（任免、改阈值、恢复权限、执行撤销）：5 分钟内没再验证过就先弹后台 Passkey，再执行 `run`；
   * 服务端仍回 `ADMIN_REVERIFY_REQUIRED`（时钟误差等）时再验证一次后重试。是否允许永远由服务端判断。
   */
  sensitive: <T>(run: () => Promise<T>) => Promise<T>;
}

const Ctx = createContext<AdminSession | null>(null);

export function useAdminSession(): AdminSession {
  const s = useContext(Ctx);
  if (!s) throw new Error("useAdminSession outside SessionProvider");
  return s;
}

const CANCELLED = () => new ApiError("ADMIN_REVERIFY_REQUIRED", "已取消再验证，操作没有执行", 0);

/** 后台会话：未登录或已过期时显示登录页；任何后台请求返回 401 都当作会话失效。 */
export function SessionProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [reverify, setReverify] = useState<{ done: (ok: boolean) => void } | null>(null);
  const queryClient = useQueryClient();
  // 只用来发现「已过期」；倒计时显示由 SessionBanner 自己按秒刷新，不让整页每秒重渲染
  const now = useNowSecs(5000);
  const clockRef = useRef<SessionClock | null>(null);
  clockRef.current = state.kind === "in" ? state.clock : null;

  const adopt = useCallback(
    (view: AdminSessionView) => setState({ kind: "in", clock: clockFrom(view, nowSecs()) }),
    [],
  );

  const load = useCallback(() => {
    setState({ kind: "loading" });
    unwrap(api.GET("/api/v1/admin/session")).then(adopt, (e: ApiError) =>
      setState(e.status === 401 ? { kind: "anon", expired: null } : { kind: "error", error: e }),
    );
  }, [adopt]);

  useEffect(load, [load]);

  const expire = useCallback(
    (reason: "idle" | "absolute" | "server") => {
      queryClient.clear();
      setState({ kind: "anon", expired: reason });
    },
    [queryClient],
  );

  useEffect(
    () =>
      onApiResult((status) => {
        if (!clockRef.current) return;
        if (status === 401) expire("server");
        else if (status >= 200 && status < 300) {
          setState((s) => (s.kind === "in" ? { ...s, clock: touch(s.clock, nowSecs()) } : s));
        }
      }),
    [expire],
  );

  const phase = state.kind === "in" ? sessionPhase(state.clock, now) : null;
  const expiredBy = phase?.phase === "expired" ? phase.reason : null;
  useEffect(() => {
    if (expiredBy) expire(expiredBy);
  }, [expiredBy, expire]);

  const askReverify = useCallback(
    () =>
      new Promise<void>((resolve, reject) =>
        setReverify({
          done: (ok) => {
            setReverify(null);
            if (ok) resolve();
            else reject(CANCELLED());
          },
        }),
      ),
    [],
  );

  const sensitive = useCallback(
    async <T,>(run: () => Promise<T>): Promise<T> => {
      const clock = clockRef.current;
      if (!clock || !reverifyFresh(clock, nowSecs())) await askReverify();
      try {
        return await run();
      } catch (e) {
        if (!(e instanceof ApiError) || e.code !== "ADMIN_REVERIFY_REQUIRED") throw e;
        await askReverify();
        return run();
      }
    },
    [askReverify],
  );

  const value = useMemo<AdminSession | null>(
    () =>
      state.kind === "in"
        ? {
            clock: state.clock,
            adopt,
            sensitive,
            keepAlive: async () => adopt(await unwrap(api.GET("/api/v1/admin/session"))),
            logout: async () => {
              await unwrap(api.POST("/api/v1/admin/logout")).catch(() => undefined);
              queryClient.clear();
              setState({ kind: "anon", expired: null });
            },
          }
        : null,
    [state, adopt, sensitive, queryClient],
  );

  if (state.kind === "loading") return <LoadingState label="正在检查后台会话" />;
  if (state.kind === "error") {
    return (
      <ErrorState
        message={state.error.message}
        action={
          <Button variant="secondary" onClick={load}>
            重试
          </Button>
        }
      />
    );
  }
  if (state.kind === "anon" || !value) {
    return <LoginGate expired={state.kind === "anon" ? state.expired : null} onLogin={adopt} />;
  }
  return (
    <Ctx.Provider value={value}>
      {children}
      {reverify ? <ReverifyDialog onDone={reverify.done} /> : null}
    </Ctx.Provider>
  );
}
