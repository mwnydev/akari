"use client";

import { Button } from "@akari/ui";
import { FilePlus2, Scale } from "lucide-react";
import dynamic from "next/dynamic";
import { useState } from "react";

// 表单库与弹窗点开时才加载（订单页首次加载 JS 预算）
const AppealDialog = dynamic(() => import("./AppealDialog"), { ssr: false });
const StatementForm = dynamic(() => import("./StatementForm"), { ssr: false });

/** 当事人的两个入口：举证期内「提交陈述 / 证据」，公示后的申诉期内「申诉」（以服务端给的 actions 为准）。 */
export function PartyCaseActions({
  caseId,
  statement,
  appeal,
  onChange,
}: {
  caseId: number;
  statement: boolean;
  appeal: boolean;
  onChange: () => void;
}) {
  const [writing, setWriting] = useState(false);
  const [appealing, setAppealing] = useState(false);
  if (!statement && !appeal) return null;
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-2">
        {statement && !writing ? (
          <Button variant="secondary" onClick={() => setWriting(true)}>
            <FilePlus2 size={16} aria-hidden />
            提交陈述 / 证据
          </Button>
        ) : null}
        {appeal ? (
          <Button variant="secondary" onClick={() => setAppealing(true)}>
            <Scale size={16} aria-hidden />
            申诉
          </Button>
        ) : null}
      </div>
      {statement && writing ? (
        <StatementForm
          caseId={caseId}
          onDone={() => {
            setWriting(false);
            onChange();
          }}
        />
      ) : null}
      {appealing ? (
        <AppealDialog
          caseId={caseId}
          onClose={() => setAppealing(false)}
          onDone={() => {
            setAppealing(false);
            onChange();
          }}
        />
      ) : null}
    </div>
  );
}
