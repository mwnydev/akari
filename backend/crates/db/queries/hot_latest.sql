-- 首页热门：只取访问者看得到的板块，走 hot_topics_all（看不到的板块很少，边扫边过滤）。
-- $1 forum_ids, $2 游标分数, $3 游标主题 ID（都不含）, $4 limit
SELECT topic_id, score
FROM hot_topics
WHERE forum_id = ANY($1) AND (score, topic_id) < ($2, $3)
ORDER BY score DESC, topic_id DESC
LIMIT $4
