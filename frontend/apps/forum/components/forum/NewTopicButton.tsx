"use client";

import { Button } from "@akari/ui";
import { Plus } from "lucide-react";
import { prefetchComposer } from "./BottomComposer";
import { useComposer } from "./ComposerContext";

/** 列表顶部的「发新帖」：打开编辑器弹窗（需要外层 ComposerProvider；指针移入时预取编辑器）。 */
export function NewTopicButton() {
  const composer = useComposer();
  if (!composer) return null;
  return (
    <Button
      size="sm"
      aria-haspopup="dialog"
      onPointerEnter={prefetchComposer}
      onFocus={prefetchComposer}
      onClick={() => composer.openComposer()}
    >
      <Plus size={16} aria-hidden />
      发新帖
    </Button>
  );
}
