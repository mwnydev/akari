"use client";

import { createContext, useContext, type ReactNode } from "react";

const Preview = createContext(false);

/**
 * 设计稿模式：/design/v2 用正式组件渲染模拟数据。打开后，发帖、点赞、评论、管理等写操作只改页面里的状态，
 * 不请求接口（设计稿里的主题 ID 是假的，绝不能落到真实数据上）。
 */
export function PreviewProvider({ children }: { children: ReactNode }) {
  return <Preview.Provider value>{children}</Preview.Provider>;
}

export function usePreview(): boolean {
  return useContext(Preview);
}
