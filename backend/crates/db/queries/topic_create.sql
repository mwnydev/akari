-- 发主题：主题 + 1 楼 + 原稿 + @提及通知，一条语句完成（post id 先取自序列，主题行直接写 last_post_id）。
-- $1 forum_id, $2 author_id, $3 title, $4 html, $5 plain_text, $6 source, $7 mentions
WITH pid AS (
  SELECT nextval(pg_get_serial_sequence('posts', 'id')) AS id
), t AS (
  INSERT INTO topics (forum_id, author_id, title, last_post_id, last_poster_id, last_post_at)
  SELECT $1, $2, $3, pid.id, $2, now() FROM pid
  RETURNING id, last_post_at
), p AS (
  INSERT INTO posts (id, topic_id, floor, author_id, html, plain_text)
  OVERRIDING SYSTEM VALUE
  SELECT pid.id, t.id, 1, $2, $4, $5 FROM pid, t
  RETURNING id
), s AS (
  INSERT INTO post_sources (post_id, source) SELECT p.id, $6 FROM p
), m AS (
  INSERT INTO outbox (kind, payload)
  SELECT 'notify.mention', jsonb_build_object('to', u, 'from', $2, 'topic_id', t.id, 'post_id', p.id, 'floor', 1)
  FROM t, p, unnest($7::bigint[]) AS u
  WHERE u <> $2
)
SELECT t.id AS "topic_id!", p.id AS "post_id!", t.last_post_at AS "last_post_at!"
FROM t, p
