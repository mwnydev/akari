"use client";

import type { Schemas } from "@akari/api-client";
import { LoadingState } from "@akari/ui";
import dynamic from "next/dynamic";

// 表单里有 Tiptap，只能在浏览器里创建编辑器
const ListingForm = dynamic(() => import("./ListingForm"), {
  ssr: false,
  loading: () => <LoadingState rows={3} label="正在加载编辑器" />,
});

export function ListingFormLoader(props: { forumSlug: string; params: Schemas["TradeParams"] }) {
  return <ListingForm {...props} />;
}
