-- 拍卖提醒（plan 第 6 节「拍卖」）：worker 定时发出的「即将结束」（给出过价的人）与「付款期快到了」（给当前一轮的
-- 中标者）提醒，每条只发一次。这里记下已发的，重复巡检、防狙击顺延都不会再发。通知仍是 `auction` 类型
-- （payload 的 `event` 区分），通知类型约束不用改。
--
-- 数据库账号（deploy/ansible 授权）：GRANT SELECT, INSERT ON auction_reminders TO forum_worker;
--   GRANT SELECT ON auction_defaults TO forum_api;   -- 名片与个人主页公开显示违约次数与违约禁拍

-- kind：1 即将结束（round 为 0）、2 付款提醒（round 为成交证明的轮次，递补者另算一次）。
CREATE TABLE auction_reminders (
  auction_id bigint      NOT NULL REFERENCES auctions (id),
  kind       smallint    NOT NULL CHECK (kind IN (1, 2)),
  round      smallint    NOT NULL CHECK (round BETWEEN 0 AND 3),
  user_id    bigint      NOT NULL REFERENCES users (id),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (auction_id, kind, round, user_id),
  CHECK ((kind = 1) = (round = 0))
);
REVOKE UPDATE, DELETE, TRUNCATE ON auction_reminders FROM PUBLIC;
