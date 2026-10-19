/** 顶栏左侧的站名（色块字标 + 「Akari」），回首页。普通链接：出错页也用它，不必为它加载客户端路由。 */
export function Brand({ href = "/" }: { href?: string }) {
  return (
    <a
      href={href}
      className="ml-1 flex items-center gap-2 rounded-sm text-lg font-semibold focus-visible:outline-2 focus-visible:outline-accent"
    >
      <span
        aria-hidden
        className="flex size-7 items-center justify-center rounded-md bg-accent-fill text-sm font-bold text-accent-fg"
      >
        A
      </span>
      Akari
    </a>
  );
}
