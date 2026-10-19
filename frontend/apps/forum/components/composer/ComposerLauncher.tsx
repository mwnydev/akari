"use client";

import { Button, LoadingState } from "@akari/ui";
import { PenLine } from "lucide-react";
import dynamic from "next/dynamic";
import { useEffect, useState } from "react";
import { loadDraft } from "../../lib/drafts";
import type { UserCardView } from "../../lib/display";
import { draftKey, type ComposeTarget } from "./target";
import { useAfterSubmit } from "./useAfterSubmit";

// Tiptap 只在点开编辑器时下载（plan 第 12 节首屏 JS 预算）；指针移上按钮时提前预取。
const loadComposer = () => import("./Composer");
const Composer = dynamic(loadComposer, {
  ssr: false,
  loading: () => <LoadingState rows={2} label="正在加载编辑器" />,
});

/** 发主题 / 回帖入口：先只渲染一个按钮，点开才加载编辑器。本机有草稿时提示继续编辑。 */
export function ComposerLauncher({
  target,
  label,
  mentionable = [],
}: {
  target: Exclude<ComposeTarget, { kind: "edit" }>;
  label: string;
  mentionable?: UserCardView[];
}) {
  const [open, setOpen] = useState(false);
  const [hasDraft, setHasDraft] = useState(false);
  const afterSubmit = useAfterSubmit();
  const key = draftKey(target);

  // localStorage 只在浏览器里有：首屏按无草稿渲染，挂载后再看
  useEffect(() => setHasDraft(loadDraft(key) !== null), [key, open]);

  if (open) {
    return (
      <Composer
        target={target}
        mentionable={mentionable}
        onCancel={() => setOpen(false)}
        onDone={(href) => {
          setOpen(false);
          afterSubmit(href);
        }}
      />
    );
  }
  return (
    <Button
      onClick={() => setOpen(true)}
      onPointerEnter={() => void loadComposer()}
      onFocus={() => void loadComposer()}
    >
      <PenLine size={18} aria-hidden />
      {hasDraft ? "继续编辑草稿" : label}
    </Button>
  );
}
