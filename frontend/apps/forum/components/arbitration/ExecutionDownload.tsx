"use client";

import type { Schemas } from "@akari/api-client";
import { Button } from "@akari/ui";
import { Download } from "lucide-react";

/** 本案的 Safe「Transaction Builder」文件（签名人导入后各自签名；每天的批量文件在后台下载）。 */
export function ExecutionDownload({ execution }: { execution: Schemas["SafeExecution"] }) {
  return (
    <Button
      variant="secondary"
      size="sm"
      onClick={() => {
        const blob = new Blob([JSON.stringify(execution.transaction_builder, null, 2)], {
          type: "application/json",
        });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `akari-arbitration-case-${execution.case_id}.json`;
        a.click();
        URL.revokeObjectURL(url);
      }}
    >
      <Download size={16} aria-hidden />
      下载本案 Safe 交易文件
    </Button>
  );
}
