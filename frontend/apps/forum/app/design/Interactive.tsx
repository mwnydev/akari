"use client";

import { Button } from "@akari/ui";
import { SegmentedControl } from "@akari/ui/tabs";
import { SignConfirmDialog } from "@akari/ui/sign-confirm";
import { Toaster, toast } from "@akari/ui/toast";
import { useState } from "react";

export function Interactive() {
  const [range, setRange] = useState<"all" | "trade" | "digest">("all");
  const [open, setOpen] = useState(false);
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Toaster position="top-center" />
      <SegmentedControl
        label="筛选"
        value={range}
        onValueChange={setRange}
        options={[
          { value: "all", label: "全部" },
          { value: "trade", label: "交易" },
          { value: "digest", label: "精华" },
        ]}
      />
      <Button onClick={() => setOpen(true)}>打开签名确认</Button>
      <Button variant="secondary" onClick={() => toast.success("已复制充值地址")}>
        提示条
      </Button>
      <SignConfirmDialog
        open={open}
        onOpenChange={setOpen}
        action="担保付款"
        amount="45050000"
        to="0x7a3b91c04e2d5f68a9b1c3d5e7f9a1b3c5d7e9f1"
        toLabel="担保合约"
        rows={[
          { label: "货款", value: "45.00 USDT" },
          { label: "代付费", value: "0.050000 USDT" },
        ]}
        onConfirm={async () => {
          await new Promise((r) => setTimeout(r, 900));
          setOpen(false);
          toast.success("付款已提交，等待上链");
        }}
      />
    </div>
  );
}
