"use client";

import { Button } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { toast } from "@akari/ui/toast";
import { useState } from "react";

export function OrderActions({ mode, id }: { mode: "once" | "stream"; id: string }) {
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="flex flex-wrap gap-2">
      {mode === "once" ? (
        <Button onClick={() => setConfirming(true)}>确认收货</Button>
      ) : (
        <Button onClick={() => toast.success("领取已提交")}>领取已解锁部分</Button>
      )}
      <Button variant="secondary" onClick={() => toast("已向对方发出和解提议，72 小时内有效")}>
        协商和解
      </Button>
      <Button variant="ghost" asChild>
        <a href={`/orders/${id}/dispute`}>发起争议</a>
      </Button>
      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent title="确认收货？" description="确认后货款立即放给卖家，不能撤回">
          <div className="flex justify-end gap-2">
            <Button variant="secondary" onClick={() => setConfirming(false)}>
              再等等
            </Button>
            <Button
              onClick={() => {
                setConfirming(false);
                toast.success("已确认收货，正在放款");
              }}
            >
              确认，放款给卖家
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
