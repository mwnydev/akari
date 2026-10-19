"use client";

import type { Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import dynamic from "next/dynamic";
import { useState } from "react";
import { can, userSanctionChoices } from "../../lib/staff";
import { useAfterSubmit } from "../composer/useAfterSubmit";
import type { ResolveAction, ResolveTarget } from "./ResolveDialog";

const ResolveDialog = dynamic(() => import("./ResolveDialog"), { ssr: false });

/**
 * 一件待处理举报的处理按钮：驳回、隐藏（楼层；1 楼为隐藏主题）、删除、处罚作者。按 /me 的权限只给内容现在所在板块
 * 能做的（服务端照常再校验）；处理后刷新页面。
 */
export function ReportActions({
  caseId,
  target,
  author,
  staff,
}: {
  caseId: number;
  target: ResolveTarget & { state: Schemas["ContentState"] };
  author: { id: number; name: string };
  staff: Schemas["StaffView"] | null;
}) {
  const [action, setAction] = useState<ResolveAction | null>(null);
  const afterSubmit = useAfterSubmit();
  if (!staff) return null;
  const f = target.forumId;
  const hide =
    target.kind === "post" &&
    target.state === "normal" &&
    can(staff, target.floor === 1 ? "topic_hide" : "post_hide", f);
  const del = target.state !== "deleted" && can(staff, "post_delete", f);
  const sanction = userSanctionChoices(staff, f).length > 0;
  return (
    <div className="flex flex-wrap gap-2" role="group" aria-label="处理举报">
      <Button size="sm" variant="secondary" onClick={() => setAction("dismiss")}>
        驳回
      </Button>
      {hide ? (
        <Button size="sm" variant="secondary" onClick={() => setAction("hide")}>
          隐藏
        </Button>
      ) : null}
      {del ? (
        <Button size="sm" variant="secondary" onClick={() => setAction("delete")}>
          删除
        </Button>
      ) : null}
      {sanction ? (
        <Button size="sm" variant="danger" onClick={() => setAction("sanction")}>
          处罚作者
        </Button>
      ) : null}
      {action ? (
        <ResolveDialog
          caseId={caseId}
          action={action}
          target={target}
          author={author}
          staff={staff}
          onClose={() => setAction(null)}
          onDone={() => {
            setAction(null);
            afterSubmit(null);
          }}
        />
      ) : null}
    </div>
  );
}
