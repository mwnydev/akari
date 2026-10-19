"use client";

import { LoadingState } from "@akari/ui";
import dynamic from "next/dynamic";

// 案件页的交互部分（表单库、弹窗、WebAuthn）在页面可交互后再加载，不计入首次加载 JS（plan 第 12 节预算 200 KB）。
const loading = () => <LoadingState rows={1} />;

export const LazyRulingForm = dynamic(() => import("./RulingForm"), { ssr: false, loading });
export const LazyOwnerControls = dynamic(() => import("./OwnerControls"), { ssr: false, loading });
export const LazyCasePartyActions = dynamic(() => import("./CasePartyActions"), {
  ssr: false,
  loading,
});
