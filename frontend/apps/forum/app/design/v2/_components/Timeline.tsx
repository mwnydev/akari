"use client";

import { ChevronsDown, ChevronsUp } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

/** 滑块手柄高度（rem），用于把楼层换算成位置。 */
const HANDLE_REM = 2.75;

/**
 * 右侧楼层时间轴（桌面端，没有卡片底）：一条细竖线，手柄显示「2 / 6」和该楼时间，两端是首尾时间。
 * 滚动时跟随当前楼层；拖动手柄、点轨道或用方向键 / PageUp / Home / End 跳楼。
 * 楼层元素约定为 `[data-floor]`，与 Floor 组件一致。
 */
export function Timeline({
  posts,
}: {
  /** 楼层号（升序）与发帖时间 */
  posts: { floor: number; time: string }[];
}) {
  const floors = useMemo(() => posts.map((p) => p.floor), [posts]);
  const total = floors.length;
  const [index, setIndex] = useState(0);
  const track = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  useEffect(() => {
    const els = document.querySelectorAll<HTMLElement>("[data-floor]");
    const io = new IntersectionObserver(
      (entries) => {
        if (dragging.current) return;
        const hit = entries.find((e) => e.isIntersecting);
        if (!hit) return;
        const i = floors.indexOf(Number((hit.target as HTMLElement).dataset.floor));
        if (i >= 0) setIndex(i);
      },
      // 视口上方 1/3 处那条线经过的楼层就是「当前楼层」
      { rootMargin: "-30% 0px -69% 0px" },
    );
    els.forEach((el) => io.observe(el));
    // 最后几层较短时到不了那条线：滚到底就算最后一层
    const onScroll = () => {
      const { scrollHeight } = document.documentElement;
      if (!dragging.current && window.innerHeight + window.scrollY >= scrollHeight - 2) {
        setIndex(floors.length - 1);
      }
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      io.disconnect();
      window.removeEventListener("scroll", onScroll);
    };
  }, [floors]);

  const jump = (i: number) => {
    const next = Math.min(total - 1, Math.max(0, i));
    setIndex(next);
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document
      .getElementById(`floor-${floors[next]}`)
      ?.scrollIntoView({ behavior: dragging.current || reduce ? "auto" : "smooth" });
  };

  const fromPointer = (e: PointerEvent) => {
    const rect = track.current?.getBoundingClientRect();
    if (!rect || total < 2) return;
    const ratio = Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height));
    jump(Math.round(ratio * (total - 1)));
  };

  const onKey = (e: KeyboardEvent) => {
    const step: Record<string, number> = {
      ArrowUp: -1,
      ArrowLeft: -1,
      ArrowDown: 1,
      ArrowRight: 1,
      PageUp: -5,
      PageDown: 5,
    };
    if (e.key === "Home") jump(0);
    else if (e.key === "End") jump(total - 1);
    else {
      const d = step[e.key];
      if (d === undefined) return;
      jump(index + d);
    }
    e.preventDefault();
  };

  const ratio = total > 1 ? index / (total - 1) : 0;
  const btn =
    "flex items-center gap-1 rounded-sm text-xs text-fg-muted hover:text-fg focus-visible:outline-2 focus-visible:outline-accent";

  return (
    <nav aria-label="楼层导航">
      <button type="button" className={btn} onClick={() => jump(0)}>
        <ChevronsUp size={14} aria-hidden />
        {posts[0]?.time}
      </button>
      <div
        ref={track}
        className="relative my-2 ml-1.5 h-56 cursor-pointer touch-none border-l border-border"
        onPointerDown={(e) => {
          dragging.current = true;
          e.currentTarget.setPointerCapture(e.pointerId);
          fromPointer(e);
        }}
        onPointerMove={(e) => {
          if (dragging.current) fromPointer(e);
        }}
        onPointerUp={() => {
          dragging.current = false;
        }}
        onPointerCancel={() => {
          dragging.current = false;
        }}
      >
        <div
          role="slider"
          tabIndex={0}
          aria-label="跳到楼层"
          aria-orientation="vertical"
          aria-valuemin={1}
          aria-valuemax={total}
          aria-valuenow={index + 1}
          aria-valuetext={`#${floors[index]}，共 ${total} 层`}
          onKeyDown={onKey}
          className="absolute -left-0.5 flex items-center gap-2.5 rounded-sm pr-1 focus-visible:outline-2 focus-visible:outline-accent"
          style={{ top: `calc((100% - ${HANDLE_REM}rem) * ${ratio})`, height: `${HANDLE_REM}rem` }}
        >
          <span className="h-full w-[3px] bg-accent-fill" />
          <span className="amount text-sm leading-tight font-medium whitespace-nowrap">
            {index + 1} / {total}
            <span className="block text-xs font-normal text-fg-muted">{posts[index]?.time}</span>
          </span>
        </div>
      </div>
      <button type="button" className={btn} onClick={() => jump(total - 1)}>
        <ChevronsDown size={14} aria-hidden />
        {posts.at(-1)?.time}
      </button>
    </nav>
  );
}
