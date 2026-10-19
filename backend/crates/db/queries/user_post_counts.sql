-- 用户发的主题数与帖子数（未删除），走 posts_author。$1 author_id
SELECT count(*) FILTER (WHERE floor = 1) AS "topics!", count(*) AS "posts!"
FROM posts
WHERE author_id = $1 AND status = 0
