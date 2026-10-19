"use client";

import { Badge, Button, Field, Input } from "@akari/ui";
import { toast } from "@akari/ui/toast";
import { useMutation } from "@tanstack/react-query";
import { ScanFace } from "lucide-react";
import { useState } from "react";
import { userMessage, walletApi, type WithdrawAddressView } from "../lib/wallet/api";
import { chain } from "../lib/wallet/config";
import { loadWalletSdk } from "../lib/wallet/load";
import { remaining } from "../lib/wallet/order";
import { addressUsable, checkAddress, labelError } from "../lib/wallet/withdraw";

/** 改提币地址是敏感操作（plan 第 5 节）：先确保近期 Passkey 验证，再调接口。 */
async function withPasskey<T>(action: () => Promise<T>): Promise<T> {
  const sdk = await loadWalletSdk();
  await sdk.confirmWithPasskey(walletApi());
  return action();
}

function AddressRow({
  a,
  now,
  onChanged,
}: {
  a: WithdrawAddressView;
  now: number;
  onChanged: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const revoke = useMutation({
    mutationFn: () => withPasskey(() => walletApi().removeWithdrawAddress(a.id)),
    onSuccess: () => {
      toast.success(`已撤销「${a.label}」`);
      onChanged();
    },
  });
  const usable = addressUsable(a, now);
  return (
    <li className="space-y-2 py-3" data-testid="withdraw-address">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{a.label}</span>
        {usable ? (
          <Badge tone="success">可用</Badge>
        ) : (
          <Badge tone="warning">冷却中 · {remaining(a.active_at, now)}生效</Badge>
        )}
      </div>
      <p className="amount font-mono text-sm break-all text-fg-muted">{a.address}</p>
      {confirming ? (
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="danger"
            disabled={revoke.isPending}
            onClick={() => revoke.mutate()}
          >
            <ScanFace size={16} aria-hidden />
            确认撤销（Face ID）
          </Button>
          <Button size="sm" variant="secondary" onClick={() => setConfirming(false)}>
            取消
          </Button>
        </div>
      ) : (
        <Button size="sm" variant="ghost" onClick={() => setConfirming(true)}>
          撤销
        </Button>
      )}
      {revoke.isError ? (
        <p role="alert" className="text-sm text-danger">
          {userMessage(revoke.error, "撤销没有成功，请重试")}
        </p>
      ) : null}
    </li>
  );
}

/**
 * 提币地址管理：新增、撤销都要 Passkey 确认；第一个地址立即可用，之后新增的 24 小时后生效，冷却中可撤销
 * （界面层保护，防会话被盗后偷换地址，plan 第 5 节「提币地址冷却」）。
 */
export function WithdrawAddresses({
  self,
  addresses,
  now,
  onChanged,
}: {
  /** 本钱包地址（不能提给自己） */
  self: string;
  addresses: WithdrawAddressView[];
  now: number;
  onChanged: () => void;
}) {
  const [label, setLabel] = useState("");
  const [address, setAddress] = useState("");
  const [touched, setTouched] = useState(false);
  const saved = addresses.filter((a) => a.status !== "revoked");
  const check = checkAddress(address, { self, contracts: Object.values(chain.contracts) });
  const duplicate =
    check.ok && saved.some((a) => a.address.toLowerCase() === check.address.toLowerCase());
  const addressError = !check.ok ? check.error : duplicate ? "这个地址已经保存过了" : null;
  const nameError = labelError(label);

  const add = useMutation({
    mutationFn: (req: { label: string; address: string }) =>
      withPasskey(() => walletApi().addWithdrawAddress(req)),
    onSuccess: (created) => {
      toast.success(
        Date.parse(created.active_at) <= Date.now()
          ? "已保存，现在就可以提回"
          : "已保存，24 小时后生效，期间可以撤销",
      );
      setLabel("");
      setAddress("");
      setTouched(false);
      onChanged();
    },
  });

  return (
    <div className="space-y-3">
      {saved.length > 0 ? (
        <ul className="divide-y divide-border">
          {saved.map((a) => (
            <AddressRow key={a.id} a={a} now={now} onChanged={onChanged} />
          ))}
        </ul>
      ) : (
        <p className="text-sm text-fg-muted">还没有保存提币地址。</p>
      )}
      <form
        className="space-y-3 rounded-lg border border-border p-3"
        onSubmit={(e) => {
          e.preventDefault();
          setTouched(true);
          if (check.ok && !addressError && !nameError) {
            add.mutate({ label: label.trim(), address: check.address });
          }
        }}
      >
        <p className="text-sm font-medium">添加交易所充值地址</p>
        <Field label="备注" error={touched ? (nameError ?? undefined) : undefined}>
          <Input value={label} onChange={(e) => setLabel(e.target.value)} placeholder="如：币安" />
        </Field>
        <Field
          label="充值地址（Arbitrum One）"
          hint={
            saved.length > 0
              ? "新地址 24 小时后生效，防止账号被盗后被偷换"
              : "第一个地址保存后立即可用"
          }
          error={(touched || address !== "") && addressError ? addressError : undefined}
        >
          <Input
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            placeholder="0x…"
            autoComplete="off"
            spellCheck={false}
            className="amount font-mono text-sm"
          />
        </Field>
        <Button type="submit" disabled={add.isPending}>
          <ScanFace size={18} aria-hidden />
          {add.isPending ? "等待 Face ID…" : "保存地址（Face ID 确认）"}
        </Button>
        {add.isError ? (
          <p role="alert" className="text-sm text-danger">
            {userMessage(add.error, "保存没有成功，请重试")}
          </p>
        ) : null}
      </form>
    </div>
  );
}
