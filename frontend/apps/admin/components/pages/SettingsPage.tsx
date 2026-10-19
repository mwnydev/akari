"use client";

import { LoadingState } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import { api, unwrap } from "../../lib/api";
import { keys } from "../../lib/queries";
import { QueryView, Section } from "../Section";

const SettingsForm = dynamic(() => import("../forms/SettingsForm"), {
  ssr: false,
  loading: () => <LoadingState />,
});

export function SettingsPage() {
  const settings = useQuery({
    queryKey: keys.settings,
    queryFn: () => unwrap(api.GET("/api/v1/admin/settings/moderation")),
  });
  return (
    <Section
      title="管理操作限速与异常阈值"
      description="管理员、版主在窗口内超过限速会被拒绝；超过异常阈值会自动暂停其管理权限并告警，待站长在「权限暂停」里确认恢复。"
    >
      <QueryView query={settings}>{(view) => <SettingsForm view={view} />}</QueryView>
    </Section>
  );
}
