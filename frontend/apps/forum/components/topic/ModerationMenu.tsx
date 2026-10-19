"use client";

import type { Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import { Shield } from "lucide-react";
import dynamic from "next/dynamic";
import { useState } from "react";
import { useAfterSubmit } from "../composer/useAfterSubmit";

const ModerationDialog = dynamic(() => import("./ModerationDialog"), { ssr: false });

/** 主题管理入口（只在 can_moderate 时渲染）：置顶 / 精华 / 锁定 / 隐藏 / 移动，均需填写理由。 */
export function ModerationMenu({
  topicId,
  forumId,
  flags,
  forums,
}: {
  topicId: number;
  forumId: number;
  flags: Schemas["TopicFlags"];
  /** 「移动」的目标板块 */
  forums: Pick<Schemas["ForumView"], "id" | "name">[];
}) {
  const [open, setOpen] = useState(false);
  const afterSubmit = useAfterSubmit();
  return (
    <>
      <Button variant="secondary" size="sm" onClick={() => setOpen(true)}>
        <Shield size={16} aria-hidden />
        管理
      </Button>
      {open ? (
        <ModerationDialog
          topicId={topicId}
          forumId={forumId}
          flags={flags}
          forums={forums}
          onClose={() => setOpen(false)}
          onDone={() => {
            setOpen(false);
            afterSubmit(null);
          }}
        />
      ) : null}
    </>
  );
}
