"use client";

import { errorMessage } from "@akari/api-client";
import { Button, Card, CardTitle } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { api, unwrap } from "../../lib/api";
import { formatTime } from "../../lib/format";
import { registerPasskey } from "../../lib/passkey";
import { keys, useStaff } from "../../lib/queries";
import { QueryView, Section } from "../Section";
import { useAdminSession } from "../session/SessionProvider";
import { Table, Td } from "../Table";

function Count({ title, value, href }: { title: string; value: number | undefined; href: string }) {
  return (
    <Card className="space-y-1">
      <CardTitle>{title}</CardTitle>
      <p className="amount text-2xl font-semibold">{value ?? "…"}</p>
      <a href={href} className="text-sm text-accent hover:underline">
        查看
      </a>
    </Card>
  );
}

/** 后台 Passkey：站长至少应有 2 个（分处保管），再加一个需要 5 分钟内再验证过。 */
function Passkeys() {
  const { sensitive } = useAdminSession();
  const queryClient = useQueryClient();
  const list = useQuery({
    queryKey: keys.passkeys,
    queryFn: () => unwrap(api.GET("/api/v1/admin/passkeys")),
  });
  const add = useMutation({
    mutationFn: () => sensitive(registerPasskey),
    onSuccess: () => {
      toast.success("已添加后台 Passkey，并已推送管理员群");
      void queryClient.invalidateQueries({ queryKey: keys.passkeys });
    },
  });
  return (
    <Section
      title="后台 Passkey"
      description="建议至少 2 个，放在不同设备上；丢失全部 Passkey 只能在服务器上执行一次性找回命令。"
      actions={
        <Button size="sm" variant="secondary" onClick={() => add.mutate()} disabled={add.isPending}>
          <KeyRound size={16} aria-hidden />
          添加 Passkey
        </Button>
      }
    >
      {add.isError ? (
        <p role="alert" className="text-sm text-danger">
          {errorMessage(add.error)}
        </p>
      ) : null}
      <QueryView query={list}>
        {(data) => (
          <Table label="后台 Passkey" head={["#", "凭证", "添加时间", "最近使用"]}>
            {data.passkeys.map((p) => (
              <tr key={p.id}>
                <Td className="amount">{p.id}</Td>
                <Td className="amount text-xs">{p.credential_id.slice(0, 16)}…</Td>
                <Td>{formatTime(p.created_at)}</Td>
                <Td>{formatTime(p.last_used_at)}</Td>
              </tr>
            ))}
          </Table>
        )}
      </QueryView>
    </Section>
  );
}

export function HomePage() {
  const staff = useStaff();
  const overview = useQuery({
    queryKey: keys.arbitration(30),
    queryFn: () =>
      unwrap(api.GET("/api/v1/admin/arbitration", { params: { query: { days: 30 } } })),
  });
  const suspensions = useQuery({
    queryKey: keys.suspensions,
    queryFn: () => unwrap(api.GET("/api/v1/admin/suspensions")),
  });
  return (
    <>
      <div className="grid gap-3 sm:grid-cols-3">
        <Count
          title="需要关注的仲裁案件"
          value={overview.data?.cases.length}
          href="/arbitration/"
        />
        <Count
          title="被暂停管理权限"
          value={suspensions.data?.suspensions.length}
          href="/suspensions/"
        />
        <Count title="在任管理人员" value={staff.data?.staff.length} href="/staff/" />
      </div>
      <Passkeys />
    </>
  );
}
