-- 重算热门排名（forum-worker 每 5 分钟，在替换整张 hot_topics 的事务里）：近 14 天内新建的普通主题，
-- 热度 = 回复数 + 1 楼点赞数 × 2 + 1，按发帖后的小时数衰减（重力 1.5，即 Hacker News 的做法）；每个板块只留前 $1 个。
-- 14 天内新建的主题最后回复也在 14 天内：多写的 last_post_at 条件让规划器走部分索引 topics_latest（范围扫描），
-- 不碰更早的主题。1 楼点赞按 (topic_id, floor) 唯一索引与 post_counters 主键各取一行，不汇总全部楼层。$1 每板块上限
INSERT INTO hot_topics (topic_id, forum_id, score)
SELECT r.id, r.forum_id, r.score
FROM (
  SELECT t.id, t.forum_id, s.score,
         row_number() OVER (PARTITION BY t.forum_id ORDER BY s.score DESC, t.id DESC) AS rank
  FROM topics t
  LEFT JOIN posts p ON p.topic_id = t.id AND p.floor = 1
  LEFT JOIN post_counters pc ON pc.post_id = p.id
  CROSS JOIN LATERAL (
    SELECT (t.last_floor + 2 * coalesce(pc.likes, 0))::float8
           / power(extract(epoch FROM now() - t.created_at)::float8 / 3600 + 2, 1.5) AS score
  ) s
  WHERE (t.flags & 9) = 0
    AND t.last_post_at > now() - interval '14 days'
    AND t.created_at > now() - interval '14 days'
) r
WHERE r.rank <= $1
