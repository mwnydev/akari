"use client";

import { ChevronsDown, ChevronsUp } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { topicHref } from "../../lib/links";
import { floorPage } from "../../lib/pagination";

/** 滑块手柄高度（rem），用于把楼层换算成位置。 */
const HANDLE_REM = 2.75;

export interface TimelinePoint {
  floor: number;
  /** 该楼的发帖时间（短写法） */
  time: string;
}

/** 楼层 `floor` 的时间：取不晚于它的最近一个采样点（采样点之间的楼层按前一个点显示）。 */
export function pointTime(points: TimelinePoint[], floor: number): string | undefined {
  let hit = points[0];
  for (const p of points) {
    if (p.floor > floor) break;
    hit = p;
  }
  return hit?.time;
}

/**
 * 右侧楼层时间轴（桌面端，没有卡片底）：一条细竖线，手柄显示「12 / 360」和该楼时间，两端是首尾楼层的时间。
 * 范围是整个主题（`points` 是接口 `/topics/{id}/timeline` 的采样点，含 1 楼与最后一楼）：
 * 滚动时跟随当前楼层；拖动手柄、点轨道或用方向键 / PageUp / Home / End 跳楼，
 * 本页的楼层直接滚过去，别的页的楼层（传了 `topicId` 时）在松手时跳到那一页。
 * 楼层元素约定为 `[data-floor]`，与 Floor 组件一致。
 */
export function Timeline({
  points,
  start,
  topicId,
}: {
  points: TimelinePoint[];
  /** 本页第一层（手柄的初始位置） */
  start?: number | undefined;
  topicId?: number | undefined;
}) {
  const first = points[0]?.floor ?? 1;
  const last = points.at(-1)?.floor ?? first;
  const span = last - first;
  const [floor, setFloor] = useState(start ?? first);
  const track = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  useEffect(() => {
    const els = [...document.querySelectorAll<HTMLElement>("[data-floor]")];
    const io = new IntersectionObserver(
      (entries) => {
        if (dragging.current) return;
        const hit = entries.find((e) => e.isIntersecting);
        if (hit) setFloor(Number((hit.target as HTMLElement).dataset.floor));
      },
      // 视口上方 1/3 处那条线经过的楼层就是「当前楼层」
      { rootMargin: "-30% 0px -69% 0px" },
    );
    els.forEach((el) => io.observe(el));
    // 最后几层较短时到不了那条线：滚到底就算本页最后一层
    const onScroll = () => {
      const { scrollHeight } = document.documentElement;
      const end = els.at(-1)?.dataset.floor;
      if (!dragging.current && end && window.innerHeight + window.scrollY >= scrollHeight - 2) {
        setFloor(Number(end));
      }
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      io.disconnect();
      window.removeEventListener("scroll", onScroll);
    };
  }, []);

  /** 跳到楼层 `f`：本页的滚过去；别的页的在 `commit`（松手、按键）时换页。 */
  const jump = (f: number, commit: boolean) => {
    const next = Math.min(last, Math.max(first, Math.round(f)));
    setFloor(next);
    const el = document.getElementById(`floor-${next}`);
    if (el) {
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      el.scrollIntoView({ behavior: dragging.current || reduce ? "auto" : "smooth" });
    } else if (commit && topicId !== undefined) {
      location.assign(topicHref(topicId, floorPage(next), next));
    }
  };

  const fromPointer = (e: PointerEvent, commit: boolean) => {
    const rect = track.current?.getBoundingClientRect();
    if (!rect || span < 1) return;
    const ratio = Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height));
    jump(first + ratio * span, commit);
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
    if (e.key === "Home") jump(first, true);
    else if (e.key === "End") jump(last, true);
    else {
      const d = step[e.key];
      if (d === undefined) return;
      jump(floor + d, true);
    }
    e.preventDefault();
  };

  const ratio = span > 0 ? (floor - first) / span : 0;
  const btn =
    "flex items-center gap-1 rounded-sm text-xs text-fg-muted hover:text-fg focus-visible:outline-2 focus-visible:outline-accent";

  return (
    <nav aria-label="楼层导航">
      <button type="button" className={btn} onClick={() => jump(first, true)}>
        <ChevronsUp size={14} aria-hidden />
        {points[0]?.time}
      </button>
      <div
        ref={track}
        className="relative my-2 ml-1.5 h-56 cursor-pointer touch-none border-l border-border"
        onPointerDown={(e) => {
          dragging.current = true;
          e.currentTarget.setPointerCapture(e.pointerId);
          fromPointer(e, false);
        }}
        onPointerMove={(e) => {
          if (dragging.current) fromPointer(e, false);
        }}
        onPointerUp={(e) => {
          dragging.current = false;
          fromPointer(e, true);
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
          aria-valuemin={first}
          aria-valuemax={last}
          aria-valuenow={floor}
          aria-valuetext={`#${floor}，共 ${last} 层`}
          onKeyDown={onKey}
          className="absolute -left-0.5 flex items-center gap-2.5 rounded-sm pr-1 focus-visible:outline-2 focus-visible:outline-accent"
          style={{ top: `calc((100% - ${HANDLE_REM}rem) * ${ratio})`, height: `${HANDLE_REM}rem` }}
        >
          <span className="h-full w-[3px] bg-accent-fill" />
          <span className="amount text-sm leading-tight font-medium whitespace-nowrap">
            {floor} / {last}
            <span className="block text-xs font-normal text-fg-muted">
              {pointTime(points, floor)}
            </span>
          </span>
        </div>
      </div>
      <button type="button" className={btn} onClick={() => jump(last, true)}>
        <ChevronsDown size={14} aria-hidden />
        {points.at(-1)?.time}
      </button>
    </nav>
  );
}
