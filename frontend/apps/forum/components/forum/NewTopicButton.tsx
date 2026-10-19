"use client";

import { Button } from "@akari/ui";
import { Plus } from "lucide-react";
import { useComposer } from "./ComposerContext";

/** 列表顶部的「发新帖」：展开底部编辑器（需要外层 ComposerProvider）。 */
export function NewTopicButton() {
  const composer = useComposer();
  if (!composer) return null;
  return (
    <Button size="sm" onClick={() => composer.openComposer()}>
      <Plus size={16} aria-hidden />
      发新帖
    </Button>
  );
}
