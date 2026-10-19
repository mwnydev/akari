"use client";

import { Amount, Button } from "@akari/ui";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { toast } from "@akari/ui/toast";
import { ScanFace } from "lucide-react";
import { useState } from "react";
import { ESCROW } from "../lib/mock";

export function PayButton({
  total,
  amount,
  sponsorFee,
}: {
  total: string;
  amount: string;
  sponsorFee: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button size="lg" className="w-full" onClick={() => setOpen(true)}>
        <ScanFace size={20} aria-hidden />
        付款 <Amount value={total} />
      </Button>
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action="担保付款"
        amount={total}
        to={ESCROW}
        toLabel="担保合约"
        rows={[
          { label: "货款（托管至确认收货）", value: <Amount value={amount} /> },
          { label: "代付费", value: <Amount value={sponsorFee} /> },
        ]}
        note="授权与付款合成一次 Face ID。付款成功的那一刻订单才成立；余额不足时整笔不会上链，也不扣代付费。"
        onConfirm={async () => {
          await new Promise((r) => setTimeout(r, 900));
          setOpen(false);
          toast.success("付款已提交，正在上链");
        }}
      />
    </>
  );
}
