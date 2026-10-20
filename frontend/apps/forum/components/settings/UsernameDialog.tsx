"use client";

import { Button } from "@akari/ui";
import { Dialog, DialogContent } from "@akari/ui/dialog";
import { UsernameForm } from "./UsernameForm";

const COPY = {
  welcome: {
    title: "给自己起个用户名",
    description:
      "用户名用于个人主页地址、@提及和交易；还没设置时别人看到的是「用户 编号」。以后也可以在「账号设置」里设置。",
  },
  trade: {
    title: "交易前先设置用户名",
    description: "开通交易、报价和担保付款都要先设置用户名：交易对手和仲裁员都按用户名认人。",
  },
} as const;

/** 设置用户名弹窗（按需加载）。保存后 `onSaved`；「以后再说」/ × / Esc 都是 `onClose`。 */
export default function UsernameDialog({
  userId,
  reason,
  onSaved,
  onClose,
}: {
  userId: number;
  reason: keyof typeof COPY;
  onSaved: () => void;
  onClose: () => void;
}) {
  const copy = COPY[reason];
  return (
    <Dialog open onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent title={copy.title} description={copy.description}>
        <div className="space-y-3">
          <UsernameForm id={userId} current={null} onSaved={onSaved} />
          <div className="flex justify-end">
            <Button variant="ghost" size="sm" onClick={onClose}>
              以后再说
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
