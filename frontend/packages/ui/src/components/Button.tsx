import { Slot } from "@radix-ui/react-slot";
import type { ButtonHTMLAttributes } from "react";
import { cn } from "../cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "subtle" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** 把样式交给唯一子元素（如 Next.js Link） */
  asChild?: boolean;
}

const variants: Record<ButtonVariant, string> = {
  primary: "bg-accent-fill text-accent-fg hover:brightness-110",
  secondary: "bg-surface-2 text-fg hover:brightness-95",
  ghost: "text-accent hover:bg-accent-soft",
  /** 低调的次要操作（楼层下的赞、评论）：灰字，悬停才显出底色 */
  subtle: "text-fg-muted hover:bg-surface-2 hover:text-fg aria-pressed:text-accent",
  danger: "bg-danger text-accent-fg hover:brightness-110",
};

const sizes: Record<ButtonSize, string> = {
  sm: "h-8 px-3 text-sm",
  md: "h-10 px-5",
  lg: "h-12 px-6 text-lg",
};

/** 胶囊形按钮。图标按钮也必须带文字（无障碍，plan 第 14 节）。 */
export function Button({
  variant = "primary",
  size = "md",
  asChild = false,
  className,
  type,
  ...props
}: ButtonProps) {
  const Comp = asChild ? Slot : "button";
  return (
    <Comp
      type={asChild ? undefined : (type ?? "button")}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-pill font-medium whitespace-nowrap",
        "transition-[filter,background-color] duration-150 select-none",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        "disabled:pointer-events-none disabled:opacity-45",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    />
  );
}
