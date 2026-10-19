-- 压测目标（k6 脚本读的 targets.json）：从灌好的库里抽样，固定随机种子，同一份数据每次抽到同样的目标。
--   psql "$DATABASE_URL" -qAt -f deploy/loadtest/targets.sql > .run/loadtest-data/targets.json
SELECT setseed(0.20261020);
WITH boards AS (
  SELECT f.id, f.slug, f.post_min_level, count(t.id) AS listed
  FROM forums f LEFT JOIN topics t ON t.forum_id = f.id AND (t.flags & 9) = 0
  WHERE f.view_min_level = 0
  GROUP BY f.id
),
-- 深分页游标：板块有序集合只放最新 1 万个，第 12000 名之后走数据库游标
deep AS (
  SELECT DISTINCT ON (b.id) b.slug,
         (extract(epoch FROM t.last_post_at) * 1000000)::bigint::text || '_' || t.id AS cursor
  FROM boards b
  JOIN LATERAL (
    SELECT id, last_post_at FROM topics
    WHERE forum_id = b.id AND (flags & 9) = 0
    ORDER BY last_post_at DESC, id DESC OFFSET 12000 LIMIT 1
  ) t ON true
),
visible AS (
  SELECT t.id, t.last_floor, t.forum_id, t.flags
  FROM topics t JOIN forums f ON f.id = t.forum_id
  WHERE f.view_min_level = 0 AND (t.flags & 8) = 0
),
sample AS (
  SELECT id, last_floor FROM visible ORDER BY random() LIMIT 5000
),
hot AS (
  SELECT id, last_floor FROM visible WHERE last_floor >= 1000 ORDER BY last_floor DESC LIMIT 50
),
-- 回帖、点赞的目标：可回复（未锁定、回复门槛 1 级）的主题与其中未删除的楼层
writable AS (
  SELECT v.id FROM visible v JOIN forums f ON f.id = v.forum_id
  WHERE f.reply_min_level <= 1 AND (v.flags & 4) = 0
  ORDER BY random() LIMIT 2000
),
likeable AS (
  SELECT p.id FROM writable w JOIN posts p ON p.topic_id = w.id AND p.floor = 1 AND p.status = 0
),
-- 热帖第 1 页的楼层：点赞会推送给订阅了这些热帖的 WebSocket 连接
hot_posts AS (
  SELECT p.id FROM hot h JOIN posts p ON p.topic_id = h.id AND p.floor <= 20 AND p.status = 0
)
SELECT json_build_object(
  'boards', (SELECT json_agg(json_build_object('slug', slug, 'pages', least(200, greatest(1, ceil(listed / 50.0))),
                                               'postable', post_min_level <= 1) ORDER BY id) FROM boards),
  'deep', (SELECT coalesce(json_agg(json_build_object('slug', slug, 'cursor', cursor)), '[]') FROM deep),
  'topics', (SELECT json_agg(json_build_array(id, last_floor)) FROM sample),
  'hot', (SELECT coalesce(json_agg(json_build_array(id, last_floor)), '[]') FROM hot),
  'writable', (SELECT json_agg(id) FROM writable),
  'posts', (SELECT json_agg(id) FROM likeable),
  'hot_posts', (SELECT coalesce(json_agg(id), '[]') FROM hot_posts),
  'users', (SELECT count(*) FROM users)
);
