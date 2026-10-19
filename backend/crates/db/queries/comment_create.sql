-- 发评论：插入评论、楼层评论数加一、写通知，一条语句（与回帖一样在自动提交下执行，计数行锁只持有到语句结束）。
-- 楼层已删除，或回复的评论不在这一楼 / 已删除时不写任何行。通知：楼主（评论了你的楼层）、被回复的人
-- （回复了你的评论；被回复的就是楼主时只发这一条）、被 @ 的人（不重复通知前两者）。
-- $1 post_id, $2 author_id, $3 回复的评论 ID（可空）, $4 body, $5 mentions
WITH p AS (
  SELECT id, topic_id, floor, author_id FROM posts WHERE id = $1 AND status = 0
), r AS (
  SELECT author_id FROM floor_comments WHERE id = $3 AND post_id = $1 AND status = 0
), c AS (
  INSERT INTO floor_comments (post_id, author_id, reply_to_id, body)
  SELECT p.id, $2, (SELECT author_id FROM r), $4 FROM p
  WHERE $3::bigint IS NULL OR EXISTS (SELECT 1 FROM r)
  RETURNING id, post_id, author_id, reply_to_id, body, created_at
), k AS (
  INSERT INTO post_counters AS pc (post_id, comments)
  SELECT post_id, 1 FROM c
  ON CONFLICT (post_id) DO UPDATE SET comments = pc.comments + 1
  RETURNING pc.comments
), n AS (
  INSERT INTO outbox (kind, payload)
  SELECT 'notify.comment', jsonb_build_object('to', u.to_id, 'from', $2::bigint, 'topic_id', p.topic_id, 'post_id', p.id,
                                              'floor', p.floor, 'comment_id', c.id, 'reply', u.reply)
  FROM p, c, LATERAL (VALUES (c.reply_to_id, true), (p.author_id, false)) AS u (to_id, reply)
  WHERE u.to_id IS NOT NULL AND u.to_id <> $2 AND (u.reply OR u.to_id IS DISTINCT FROM c.reply_to_id)
), m AS (
  INSERT INTO outbox (kind, payload)
  SELECT 'notify.mention', jsonb_build_object('to', u, 'from', $2::bigint, 'topic_id', p.topic_id, 'post_id', p.id,
                                              'floor', p.floor, 'comment_id', c.id)
  FROM p, c, unnest($5::bigint[]) AS u
  WHERE u <> $2 AND u <> p.author_id AND u IS DISTINCT FROM c.reply_to_id
)
SELECT c.id AS "id!", c.post_id AS "post_id!", c.author_id AS "author_id!", c.reply_to_id, c.body AS "body!",
       c.created_at AS "created_at!", k.comments AS "comments!"
FROM c, k
