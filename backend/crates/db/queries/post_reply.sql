-- 回帖（plan 第 8 节）：分配楼层、插入楼层与原稿、写发件箱，一条多 CTE 语句；在自动提交下执行，
-- 主题行锁只持有到语句结束（亚毫秒）。隐藏或锁定的主题不更新任何行（$7 为真时管理人员可回复锁定帖）。
-- $1 topic_id, $2 author_id, $3 html, $4 plain_text, $5 source, $6 mentions, $7 allow_locked
WITH pid AS (
  SELECT nextval(pg_get_serial_sequence('posts', 'id')) AS id
), t AS (
  UPDATE topics
  SET last_floor = last_floor + 1, last_post_id = pid.id, last_poster_id = $2, last_post_at = now()
  FROM pid
  WHERE topics.id = $1 AND (topics.flags & 8) = 0 AND ((topics.flags & 4) = 0 OR $7)
  RETURNING topics.id, topics.forum_id, topics.author_id, topics.flags, topics.last_floor, topics.last_post_at
), p AS (
  INSERT INTO posts (id, topic_id, floor, author_id, html, plain_text)
  OVERRIDING SYSTEM VALUE
  SELECT pid.id, t.id, t.last_floor, $2, $3, $4 FROM pid, t
  RETURNING id, floor
), s AS (
  INSERT INTO post_sources (post_id, source) SELECT p.id, $5 FROM p
), n AS (
  INSERT INTO outbox (kind, payload)
  SELECT 'notify.reply', jsonb_build_object('to', t.author_id, 'from', $2, 'topic_id', t.id, 'post_id', p.id, 'floor', p.floor)
  FROM t, p
  WHERE t.author_id <> $2
), m AS (
  INSERT INTO outbox (kind, payload)
  SELECT 'notify.mention', jsonb_build_object('to', u, 'from', $2, 'topic_id', t.id, 'post_id', p.id, 'floor', p.floor)
  FROM t, p, unnest($6::bigint[]) AS u
  WHERE u <> $2 AND u <> t.author_id
)
SELECT p.id AS "post_id!", p.floor AS "floor!", t.forum_id AS "forum_id!", t.flags AS "flags!",
       t.last_post_at AS "last_post_at!"
FROM t, p
