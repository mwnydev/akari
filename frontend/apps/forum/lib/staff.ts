// 论坛里的管理入口按 GET /api/v1/me 的 `staff` 显示（ADR 0013）。这里只决定「显示什么」：
// 每个管理接口在服务端都会再校验权限、范围与时长，界面上被隐藏的操作也不会因为绕过界面而生效，403 照常提示。

import type { Me, Schemas } from "@akari/api-client";

export type Permission = Schemas["Permission"];
export type SanctionKind = Schemas["SanctionKind"];
type Staff = NonNullable<Me["staff"]>;

/** 能否在 `forumId` 板块（不传 = 全站）做 `p`。暂停期间两组权限列表都为空。 */
export function can(staff: Staff | null | undefined, p: Permission, forumId?: number): boolean {
  if (!staff || staff.suspended) return false;
  if (staff.site_permissions.includes(p)) return true;
  return (
    forumId !== undefined && staff.boards.includes(forumId) && staff.board_permissions.includes(p)
  );
}

/** 能处理举报（版主在自己的板块，管理员与站长全站）：顶栏账号菜单里的「举报处理」入口。 */
export function canHandleReports(staff: Staff | null | undefined): boolean {
  return can(staff, "reports") || (staff?.boards ?? []).some((b) => can(staff, "reports", b));
}

/** 能接仲裁案件（站长、管理员）。 */
export function isArbitrator(staff: Staff | null | undefined): boolean {
  return Boolean(staff?.roles.some((r) => r === "owner" || r === "admin"));
}

const PERMISSION_OF: Record<SanctionKind, Permission> = {
  mute: "mute",
  auction_ban: "auction_ban",
  ban: "ban",
  ip_ban: "ip_ban",
  device_ban: "device_ban",
};

export const SANCTION_LABEL: Record<SanctionKind, string> = {
  mute: "禁言",
  auction_ban: "禁拍",
  ban: "封号",
  device_ban: "封禁登录过的设备",
  ip_ban: "封禁 IP / 网段",
};

/** 只有禁言、禁拍能限定板块；其余只能全站。 */
export function boardScoped(kind: SanctionKind): boolean {
  return kind === "mute" || kind === "auction_ban";
}

/**
 * 访问者在 `forumId` 板块能用的处罚，及其范围：全站权限的人可以选全站或本板块；
 * 只有本板块权限（版主）的只能限本板块。
 */
export function sanctionChoices(
  staff: Staff | null | undefined,
  forumId: number | undefined,
): { kind: SanctionKind; site: boolean; board: boolean }[] {
  return (Object.keys(PERMISSION_OF) as SanctionKind[]).flatMap((kind) => {
    const p = PERMISSION_OF[kind];
    const site = can(staff, p);
    const board = boardScoped(kind) && forumId !== undefined && can(staff, p, forumId);
    return site || board ? [{ kind, site, board }] : [];
  });
}

/** 针对用户本人的处罚（举报处理里处罚作者用；IP 封禁不针对某个用户）。 */
export function userSanctionChoices(
  staff: Staff | null | undefined,
  forumId: number,
): ReturnType<typeof sanctionChoices> {
  return sanctionChoices(staff, forumId).filter((c) => c.kind !== "ip_ban");
}

const HOUR = 3600;
const DAY = 86_400;

/** 可选时长（秒；null = 永久），按 plan 第 9 节常用档位。 */
export const DURATIONS: { secs: number | null; label: string }[] = [
  { secs: HOUR, label: "1 小时" },
  { secs: DAY, label: "1 天" },
  { secs: 3 * DAY, label: "3 天" },
  { secs: 7 * DAY, label: "7 天" },
  { secs: 30 * DAY, label: "30 天" },
  { secs: 90 * DAY, label: "90 天" },
  { secs: 365 * DAY, label: "1 年" },
  { secs: null, label: "永久" },
];

/** 处罚时长上限（秒；null = 不限，可永久）。版主禁言 ≤ 7 天、禁拍 ≤ 30 天，管理员与站长不限。 */
export function durationLimit(staff: Staff | null | undefined, kind: SanctionKind): number | null {
  if (!staff) return 0;
  if (kind === "mute") return staff.max_mute_secs ?? null;
  if (kind === "auction_ban") return staff.max_auction_ban_secs ?? null;
  return null;
}

/** 在上限内的时长档位。 */
export function durationChoices(limit: number | null): typeof DURATIONS {
  return DURATIONS.filter((d) => limit === null || (d.secs !== null && d.secs <= limit));
}
