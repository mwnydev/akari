"use client";

import { Button, Field } from "@akari/ui";
import { LazyMenu } from "@akari/ui/lazy";
import {
  Ban,
  EyeOff,
  FolderInput,
  Lock,
  Pin,
  Shield,
  Star,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { boards } from "../_mock/data";
import { selectClass } from "./styles";

const ReasonDialog = lazy(() => import("./ReasonDialog"));

type Action = "pin" | "feature" | "lock" | "hide" | "move" | "delete" | "mute";

const ACTIONS: {
  value: Action;
  label: string;
  confirm: string;
  Icon: LucideIcon;
  danger: boolean;
}[] = [
  { value: "pin", label: "置顶", confirm: "置顶", Icon: Pin, danger: false },
  { value: "feature", label: "设为精华", confirm: "设为精华", Icon: Star, danger: false },
  { value: "lock", label: "锁帖（禁止回复）", confirm: "锁定", Icon: Lock, danger: false },
  { value: "hide", label: "隐藏", confirm: "隐藏", Icon: EyeOff, danger: false },
  { value: "move", label: "移动到其他板块", confirm: "移动", Icon: FolderInput, danger: false },
  { value: "delete", label: "删除主题", confirm: "删除", Icon: Trash2, danger: true },
  { value: "mute", label: "禁言作者", confirm: "禁言", Icon: Ban, danger: true },
];

/** 主题页的管理入口（ADR 0013：版主与管理员就在论坛里操作，没有单独的「管理模式」）。 */
export function ModMenu({ board, author }: { board: string; author: string }) {
  const [action, setAction] = useState<Action | null>(null);
  const current = ACTIONS.find((a) => a.value === action);
  const labelOf = (a: (typeof ACTIONS)[number]) =>
    a.value === "mute" ? `禁言 ${author}` : a.label;
  return (
    <>
      <LazyMenu
        align="end"
        heading="版主操作 · 需填理由"
        trigger={
          <Button variant="secondary" size="sm">
            <Shield size={16} aria-hidden />
            管理
          </Button>
        }
        items={ACTIONS.map((a) => ({
          label: labelOf(a),
          icon: <a.Icon size={16} aria-hidden />,
          danger: a.danger,
          separatorBefore: a.value === "delete",
          onSelect: () => setAction(a.value),
        }))}
      />
      {current ? (
        <Suspense fallback={null}>
          <ReasonDialog
            open
            onOpenChange={(o) => {
              if (!o) setAction(null);
            }}
            title={labelOf(current)}
            confirmLabel={current.confirm}
            danger={current.danger}
          >
            {current.value === "move" ? (
              <Field label="目标板块">
                <select className={selectClass}>
                  {boards
                    .filter((b) => b.slug !== board)
                    .map((b) => (
                      <option key={b.slug}>{b.name}</option>
                    ))}
                </select>
              </Field>
            ) : null}
            {current.value === "mute" ? (
              <Field label="时长" hint="版主最长 7 天；更长或封号由管理员处理">
                <select className={selectClass} defaultValue="1 天">
                  {["1 小时", "1 天", "3 天", "7 天"].map((d) => (
                    <option key={d}>{d}</option>
                  ))}
                </select>
              </Field>
            ) : null}
          </ReasonDialog>
        </Suspense>
      ) : null}
    </>
  );
}
