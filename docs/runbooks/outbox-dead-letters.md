# 发件箱死信

站内通知、实时推送、邮件、管理员群 Telegram 告警都经事务发件箱（`outbox` 表）由 forum-worker 投递（plan 第 2 节）。
第 9 轮整合时一条违反约束的通知（新增的 `auction` 类型没加进 CHECK）让整批事务回滚，所有通知和邮件都停了；
现在每条消息各自成败，坏消息进死信，不挡别的消息。

## 投递规则

- **认领**：worker 用 `FOR UPDATE SKIP LOCKED` 认领一批（10 条），`attempts + 1`，`next_attempt_at` 推后 20 分钟（租期）。
  可以跑多个 worker 实例；认领后崩溃的消息租期一过重新到期。`attempts` 是认领令牌：租期过后被别人重新认领的，原认领者的标记不生效。
- **逐条投递**：站内通知与「已发送」标记在这条消息自己的事务里，失败只回滚它自己；邮件与 Telegram 发出后再标记。单条限时 60 秒。
- **失败分类**：
  - 重试也不会成功 → **立即进死信**：违反约束（SQLSTATE 23）、数据格式错误（22）、读出的行对不上代码、payload 不对、不认识的类型、
    服务商明确拒收（Postmark 300 / 406）。
  - 其余 → 退避重试：2^n 秒 × 0.5–1 随机系数，最长 1 小时；第 16 次仍失败进死信（累计约 2.5–5 小时，扛得住服务商短时故障）。
    数据库连接、并发冲突、权限不足、网络、服务商 5xx / 429、超时、Telegram 的任何错误都算这一类。
- **顺序**：同一订单的 `order.updated` 按 ID 顺序投递（推送里带订单状态，乱序会让页面停在旧状态）；前一条在退避中时后面的等着，
  只挡这一个订单。前一条进了死信就不再挡。其他消息不保证顺序（通知按 ID 展示，偶尔先后颠倒无妨）。
- **重复**：发出邮件 / Telegram 之后、标记之前出错（或超过租期）会再发一次。Resend 按幂等键 `outbox-<ID>` 24 小时内只投递一次；
  Postmark 没有幂等键，可能重复；Telegram 告警可能重复。站内通知与标记同一事务，不会重复。
- **错误信息**：`last_error`、日志、告警里只有错误类型与 PostgreSQL 主消息（约束名），像邮箱地址的词替换为 `<redacted>`，截到 300 字；
  不记 payload。

## 发现

- 管理员群：`[outbox.dead_letter.<类型>] 发件箱消息 #ID（类型）尝试 N 次后进了死信：<错误>`。同一类型一小时只提醒一次；
  `staff_alert` 自己进死信时不提醒（Telegram 多半不可用），只写 error 日志 `outbox message dead-lettered`。
- 日志：worker 每 5 分钟写一行 `outbox status`（待发、死信、过期丢弃条数）；有死信或最早待发消息超过 10 分钟的类型另写
  warn `outbox kind needs attention`（kind / pending / retrying / dead / oldest_pending_secs）。
- 后台「发件箱」页（`GET /api/v1/admin/outbox`）：每种消息的待发、重试中、死信条数与死信列表（不含 payload）。

## 处理

1. 看死信的类型与错误，找到原因：约束与代码不一致 → 发迁移修约束；配置错误（发件域名、令牌、权限）→ 改配置；数据本身有问题 → 丢弃。
2. 修好之后在后台「发件箱」重新投递（`POST /api/v1/admin/outbox/requeue`，一次最多 100 条）：尝试次数清零、立即到期、payload 原样。
   不要求后台 Passkey 再验证（发件箱不涉及资金），写审计日志 `outbox.requeue`。
3. 不需要再投递的丢弃（`POST /api/v1/admin/outbox/discard`）：清空 payload，审计日志 `outbox.discard`。

注意：

- 重新投递很久以前的 `order.updated` 会把旧状态再推一次（之后的状态已经推过）；页面重新拉取订单即可恢复，一般直接丢弃。
- 验证码邮件进死信后已经过期，丢弃即可，用户会重新获取。
- `notify.likes`（合并点赞通知）进死信时 worker 已清掉这一楼的待发标记，之后的新赞照常通知；重新投递也不会重复计数。
- 死信 14 天后自动丢弃（payload 里有邮箱地址等）。

## 数据库

- 迁移 `20261220000001_outbox_dead_letters.sql`：`discarded_at`、生成列 `ordering_key`，部分索引 `outbox_ordering`（认领时判断顺序）
  与 `outbox_dead`（死信列表、统计、过期丢弃）；认领走原有的 `outbox_pending`。执行计划由 `crates/db/tests/explain.rs` 校验。
- 授权（`deploy/ansible/roles/postgresql/templates/grants.sql.j2`）：forum_api 只能读死信的类型、次数、错误与时间，改
  `attempts / next_attempt_at / failed_at / discarded_at / payload`（重新投递、丢弃），读不到 payload；forum_worker 另需 `discarded_at`。
