import type { Schemas } from "@akari/api-client";

const dateTime = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

/** 后台表格里的时间（本地时区，到分钟）。 */
export function formatTime(iso: string | null | undefined): string {
  return iso ? dateTime.format(new Date(iso)) : "—";
}

export const ROLE_LABEL: Record<Schemas["Role"], string> = {
  owner: "站长",
  admin: "管理员",
  moderator: "版主",
};

/** 仲裁案件状态（`forum_db::arbitration::STATUS_*`）。 */
export const CASE_STATUS: Record<string, string> = {
  reviewing: "裁决中",
  owner_review: "待站长终裁",
  appeal_window: "申诉期",
  appealed: "申诉复核中",
  ready: "待 Safe 执行",
  halted: "已叫停",
};

export const REVERT_STATUS: Record<Schemas["RevertStatus"], string> = {
  reverted: "可撤销",
  already_reverted: "已撤销过",
  superseded: "之后又被改过，跳过",
  not_revertible: "不可撤销",
};

/** 审计日志里的操作名（只列常见的；其余原样显示）。 */
export const ACTION_LABEL: Record<string, string> = {
  "topic.pin": "置顶",
  "topic.unpin": "取消置顶",
  "topic.feature": "设精华",
  "topic.unfeature": "取消精华",
  "topic.lock": "锁帖",
  "topic.unlock": "解锁",
  "topic.hide": "隐藏主题",
  "topic.unhide": "取消隐藏",
  "topic.move": "移动主题",
  "post.edit": "编辑楼层",
  "post.delete": "删除楼层",
  "post.restore": "恢复楼层",
  "user.mute": "禁言",
  "user.auction_ban": "禁拍",
  "user.ban": "封号",
  "user.device_ban": "封设备",
  "ip.ban": "封 IP",
  "sanction.lift": "解除处罚",
  "staff.appoint": "任命",
  "staff.remove": "免职",
  "staff.reinstate": "恢复管理权限",
  "settings.moderation": "改限速与阈值",
  "admin.revoke_actions": "批量撤销",
  "admin.passkey_add": "绑定后台 Passkey",
  "outbox.requeue": "重新投递死信",
  "outbox.discard": "丢弃死信",
};

export function actionLabel(action: string): string {
  return ACTION_LABEL[action] ?? action;
}

/** 用户 ID → 名字；后台不能跨源调论坛名片接口，只能用任命列表里的用户名，其余显示编号。 */
export function userLabel(id: number, names: ReadonlyMap<number, string>): string {
  const name = names.get(id);
  return name ? `${name}（#${id}）` : `#${id}`;
}

/** 最小单位 → 「12.5 USDT」（不经浮点数）。 */
export function formatToken(amount: string, decimals: number): string {
  const v = BigInt(amount);
  const base = 10n ** BigInt(decimals);
  const frac = (v % base).toString().padStart(decimals, "0").replace(/0+$/, "");
  return `${v / base}${frac ? `.${frac}` : ""} USDT`;
}
