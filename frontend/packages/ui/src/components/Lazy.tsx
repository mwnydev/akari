"use client";

import { Slot } from "@radix-ui/react-slot";
import {
  lazy,
  Suspense,
  useState,
  type ButtonHTMLAttributes,
  type ReactElement,
  type ReactNode,
} from "react";
import type { LazyMenuItem } from "./LazyImpl";

export type { LazyMenuItem } from "./LazyImpl";

// 弹出层（Radix 菜单、气泡卡片、抽屉连同定位库）共约 40 KB，列表和楼层里每个头像、「…」都要用：
// 首屏只渲染外观相同的普通按钮，指针移入或获得焦点时预取，点击后才换成真正的组件（plan 第 12 节 JS 预算）。
const load = () => import("./LazyImpl");
const MenuImpl = lazy(() => load().then((m) => ({ default: m.MenuImpl })));
const PopoverImpl = lazy(() => load().then((m) => ({ default: m.PopoverImpl })));
const DrawerImpl = lazy(() => load().then((m) => ({ default: m.DrawerImpl })));

type Trigger = ReactElement<ButtonHTMLAttributes<HTMLButtonElement>>;

/** 受控或非受控的打开状态；第一次打开后保持挂载，关闭时焦点才能回到真正的触发按钮。 */
function useLazyOpen(open: boolean | undefined, onOpenChange: ((o: boolean) => void) | undefined) {
  const [inner, setInner] = useState(false);
  const [mounted, setMounted] = useState(false);
  const isOpen = open ?? inner;
  if (isOpen && !mounted) setMounted(true);
  return { isOpen, setOpen: onOpenChange ?? setInner, mounted: mounted || isOpen };
}

// 用 Slot 而不是 cloneElement 给触发按钮加属性：服务端组件传来的 `trigger` 在 next dev 下可能是 React.lazy 包装
// （RSC 调试信息让元素分块下发），cloneElement 会得到 type 为 undefined 的元素并报「Element type is invalid」；Slot 会先解开。
function placeholder(trigger: Trigger, popup: "menu" | "dialog", onOpen: () => void) {
  return (
    <Slot
      aria-haspopup={popup}
      aria-expanded={false}
      onPointerEnter={() => void load()}
      onFocus={() => void load()}
      onClick={onOpen}
    >
      {trigger}
    </Slot>
  );
}

/** 按需加载的下拉菜单。`trigger` 必须是按钮元素；`open` 传入时为受控（如长按打开）。 */
export function LazyMenu({
  trigger,
  items,
  heading,
  align = "start",
  open,
  onOpenChange,
}: {
  trigger: Trigger;
  items: LazyMenuItem[];
  heading?: string;
  align?: "start" | "end";
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const { isOpen, setOpen, mounted } = useLazyOpen(open, onOpenChange);
  const fallback = placeholder(trigger, "menu", () => setOpen(true));
  if (!mounted) return fallback;
  return (
    <Suspense fallback={fallback}>
      <MenuImpl
        trigger={trigger}
        items={items}
        heading={heading}
        align={align}
        open={isOpen}
        onOpenChange={setOpen}
      />
    </Suspense>
  );
}

/** 按需加载的气泡卡片（用户名片等）。 */
export function LazyPopover({ trigger, children }: { trigger: Trigger; children: ReactNode }) {
  const { isOpen, setOpen, mounted } = useLazyOpen(undefined, undefined);
  const fallback = placeholder(trigger, "dialog", () => setOpen(true));
  if (!mounted) return fallback;
  return (
    <Suspense fallback={fallback}>
      <PopoverImpl trigger={trigger} open={isOpen} onOpenChange={setOpen}>
        {children}
      </PopoverImpl>
    </Suspense>
  );
}

/** 按需加载的左侧抽屉（移动端板块导航）。 */
export function LazyDrawer({
  trigger,
  title,
  children,
}: {
  trigger: Trigger;
  title: string;
  children: ReactNode;
}) {
  const { isOpen, setOpen, mounted } = useLazyOpen(undefined, undefined);
  const fallback = placeholder(trigger, "dialog", () => setOpen(true));
  if (!mounted) return fallback;
  return (
    <Suspense fallback={fallback}>
      <DrawerImpl trigger={trigger} title={title} open={isOpen} onOpenChange={setOpen}>
        {children}
      </DrawerImpl>
    </Suspense>
  );
}
