"use client";

import { useRouter } from "next/navigation";
import { QueryProvider } from "../QueryProvider";
import { PartyCaseActions } from "./PartyCaseActions";

/** 案件页里的当事人入口：提交后刷新服务端渲染的案件详情。 */
export default function CasePartyActions(props: {
  caseId: number;
  statement: boolean;
  appeal: boolean;
}) {
  const router = useRouter();
  return (
    <QueryProvider>
      <PartyCaseActions {...props} onChange={() => router.refresh()} />
    </QueryProvider>
  );
}
