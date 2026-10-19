-- 板块热门：排名表里分数从高到低，按 (分数, 主题 ID) 游标翻页，走 hot_topics_forum。
-- $1 forum_id, $2 游标分数, $3 游标主题 ID（都不含）, $4 limit
SELECT topic_id, score
FROM hot_topics
WHERE forum_id = $1 AND (score, topic_id) < ($2, $3)
ORDER BY score DESC, topic_id DESC
LIMIT $4
