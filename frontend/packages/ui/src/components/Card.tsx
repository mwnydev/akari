import type { HTMLAttributes } from "react";
import { cn } from "../cn";

/** 实底内容卡片：正文、列表、金额都放在实底上，保证可读性。 */
export function Card({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("rounded-xl border border-border bg-surface p-4 sm:p-5", className)}
      {...props}
    />
  );
}

export function CardTitle({ className, ...props }: HTMLAttributes<HTMLHeadingElement>) {
  return <h2 className={cn("text-lg leading-snug font-semibold", className)} {...props} />;
}
