"use client";

import { Brand } from "../components/forum/Brand";

const button =
  "inline-flex h-8 items-center rounded-md px-3 text-sm font-medium focus-visible:outline-2 focus-visible:outline-accent";

/**
 * 渲染期间的意外错误（接口错误由各页面自己显示中文提示，走到这里的是程序缺陷或网络中断）。
 * 页面骨架可能正是出错的部分，这里只放一条不依赖数据的顶栏。不引用组件库：出错页的 JS 每个页面都要带上。
 */
export default function RouteError({ reset }: { error: Error; reset: () => void }) {
  return (
    <>
      <header className="h-14 border-b border-border bg-surface">
        <div className="mx-auto flex h-full max-w-[90rem] items-center px-2 sm:px-4">
          <Brand />
        </div>
      </header>
      <main className="mx-auto max-w-3xl px-4 py-12 text-center">
        <h1 className="text-lg font-medium">页面出了点问题</h1>
        <p role="alert" className="mt-1 text-sm text-fg-muted">
          请稍后重试；一直这样的话，回首页看看。
        </p>
        <div className="mt-4 flex justify-center gap-2">
          <button
            type="button"
            onClick={reset}
            className={`${button} border border-border bg-surface hover:bg-surface-2`}
          >
            重试
          </button>
          <a href="/" className={`${button} text-fg-muted hover:bg-surface-2 hover:text-fg`}>
            返回首页
          </a>
        </div>
      </main>
    </>
  );
}
