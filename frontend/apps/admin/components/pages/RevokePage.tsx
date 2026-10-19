"use client";

import { LoadingState } from "@akari/ui";
import dynamic from "next/dynamic";

/** 批量撤销页整页都是表单：连同表单库一起按需加载。 */
export const RevokePage = dynamic(() => import("../forms/RevokeForm"), {
  ssr: false,
  loading: () => <LoadingState />,
});
