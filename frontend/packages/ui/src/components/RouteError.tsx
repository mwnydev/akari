"use client";

const button =
  "inline-flex h-8 items-center rounded-md px-3 text-sm font-medium focus-visible:outline-2 focus-visible:outline-accent";

/**
 * 应用 `app/error.tsx` 的内容：渲染期间的意外错误（接口错误由各页面自己显示）。只用原生元素，不带组件库的其他部分：
 * 出错页的 JS 每个页面都要带上。`title` 是浏览器标签页标题（客户端组件不能导出 metadata，React 19 把 <title> 提升到 <head>）。
 */
export function RouteError({
  title,
  home,
  reset,
}: {
  title: string;
  home: string;
  reset: () => void;
}) {
  return (
    <main className="mx-auto max-w-xl px-4 py-12 text-center">
      <title>{title}</title>
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
          {home}
        </a>
      </div>
    </main>
  );
}
