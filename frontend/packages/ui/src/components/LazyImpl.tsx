"use client";

// Lazy.tsx 按需加载的真正实现；不要直接引用本文件。
import { Fragment, type ReactElement, type ReactNode } from "react";
import { Dialog, DialogTrigger, DrawerContent } from "./Dialog";
import { Menu, MenuContent, MenuItem, MenuLabel, MenuSeparator, MenuTrigger } from "./Menu";
import { Popover, PopoverContent, PopoverTrigger } from "./Popover";

export interface LazyMenuItem {
  label: string;
  icon?: ReactNode;
  danger?: boolean;
  /** 在这一项前面加分隔线 */
  separatorBefore?: boolean;
  onSelect: () => void;
}

interface OpenProps {
  trigger: ReactElement;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function MenuImpl({
  trigger,
  items,
  heading,
  align,
  open,
  onOpenChange,
}: OpenProps & { items: LazyMenuItem[]; heading: string | undefined; align: "start" | "end" }) {
  return (
    <Menu open={open} onOpenChange={onOpenChange}>
      <MenuTrigger asChild>{trigger}</MenuTrigger>
      <MenuContent align={align}>
        {heading ? <MenuLabel>{heading}</MenuLabel> : null}
        {items.map((it) => (
          <Fragment key={it.label}>
            {it.separatorBefore ? <MenuSeparator /> : null}
            <MenuItem danger={it.danger ?? false} onSelect={it.onSelect}>
              {it.icon}
              {it.label}
            </MenuItem>
          </Fragment>
        ))}
      </MenuContent>
    </Menu>
  );
}

export function PopoverImpl({
  trigger,
  open,
  onOpenChange,
  children,
}: OpenProps & { children: ReactNode }) {
  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      <PopoverContent>{children}</PopoverContent>
    </Popover>
  );
}

export function DrawerImpl({
  trigger,
  title,
  open,
  onOpenChange,
  children,
}: OpenProps & { title: string; children: ReactNode }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DrawerContent title={title}>{children}</DrawerContent>
    </Dialog>
  );
}
