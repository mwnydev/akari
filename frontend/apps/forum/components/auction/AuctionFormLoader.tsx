"use client";

import type { Schemas } from "@akari/api-client";
import { LoadingState } from "@akari/ui";
import dynamic from "next/dynamic";
import type { AuctionParams } from "../../lib/auction";

// 表单里有 Tiptap，只能在浏览器里创建编辑器
const AuctionForm = dynamic(() => import("./AuctionForm"), {
  ssr: false,
  loading: () => <LoadingState rows={3} label="正在加载编辑器" />,
});

export function AuctionFormLoader(props: {
  forumSlug: string;
  trade: Schemas["TradeParams"];
  auction: AuctionParams;
}) {
  return <AuctionForm {...props} />;
}
