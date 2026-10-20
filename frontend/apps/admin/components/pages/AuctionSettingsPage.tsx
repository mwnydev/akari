"use client";

import { LoadingState } from "@akari/ui";
import { useQuery } from "@tanstack/react-query";
import dynamic from "next/dynamic";
import { api, unwrap } from "../../lib/api";
import { keys } from "../../lib/queries";
import { QueryView, Section } from "../Section";

const AuctionSettingsForm = dynamic(() => import("../forms/AuctionSettingsForm"), {
  ssr: false,
  loading: () => <LoadingState />,
});

export function AuctionSettingsPage() {
  const settings = useQuery({
    queryKey: keys.auctionSettings,
    queryFn: () => unwrap(api.GET("/api/v1/admin/settings/auction")),
  });
  return (
    <Section
      title="拍卖开关与参数"
      description="只有站长能修改；每次修改都要 Passkey 再验证，写入审计日志并推送管理员群。只影响之后的挂拍、出价与结算：已签的拍卖授权里的期限不变。"
    >
      <QueryView query={settings}>{(view) => <AuctionSettingsForm view={view} />}</QueryView>
    </Section>
  );
}
