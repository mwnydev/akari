import { Amount } from "@akari/ui";

export default function Home() {
  return (
    <main className="mx-auto max-w-3xl px-4 py-6">
      <header className="glass sticky top-4 rounded-pill px-5 py-2 shadow-float">
        <h1 className="text-lg font-semibold">论坛</h1>
      </header>
      <section className="mt-6 rounded-xl bg-surface p-5">
        <p>论坛首页（服务端渲染）。</p>
        <p className="mt-2 text-fg-muted">
          示例金额：
          <Amount value="124124072" />
        </p>
      </section>
    </main>
  );
}
