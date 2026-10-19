-- 楼层点赞与朋友圈式评论（设计稿 v2，两层：楼层 → 平铺评论）、举报、主题列表的参与者头像。
--
-- 点赞、评论、计数都不加外键：与 posts 一样是热表，外键检查会给被赞楼层加 KEY SHARE 行锁（热帖下变成多事务锁）。
-- 楼层只软删除、从不物理删除，写入语句里按 `posts.status = 0` 判断楼层存在。
--
-- forum_worker 另需（deploy/ansible 授权）：合并点赞通知时读赞、推进水位：
--   GRANT SELECT ON post_likes, posts TO forum_worker;
--   GRANT SELECT, UPDATE (like_notify_pending, likes_notified_at) ON post_counters TO forum_worker;

-- 每楼一行计数，与点赞 / 评论在同一条语句里更新。不放在 posts 上：posts 行宽（HTML）且有 PGroonga 全文索引，
-- 每次更新都写新元组并重建各索引项；这张窄表只有主键索引，fillfactor 70 让计数更新走 HOT。
-- 同一楼层的并发点赞在这一行上排队，行锁只持有到自动提交的单条语句结束（亚毫秒），与回帖分配楼层同一做法。
CREATE TABLE post_counters (
  post_id             bigint      PRIMARY KEY,
  likes               int         NOT NULL DEFAULT 0 CHECK (likes >= 0),
  comments            int         NOT NULL DEFAULT 0 CHECK (comments >= 0),  -- 未删除的评论
  like_notify_pending boolean     NOT NULL DEFAULT false,  -- 已排了一条合并点赞通知、worker 还没投递
  likes_notified_at   timestamptz                          -- 上一条点赞通知统计到的最后一个赞
) WITH (fillfactor = 70);

CREATE TABLE post_likes (
  post_id    bigint      NOT NULL,
  user_id    bigint      NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (post_id, user_id)
);
-- 点赞名单预览（最近的几位）与合并通知（水位之后的新赞）：只读索引即可回答。
CREATE INDEX post_likes_recent ON post_likes (post_id, created_at DESC) INCLUDE (user_id);

-- 楼层评论：纯文本（服务端已规范化，前端按文本渲染），回复某条评论时记下被回复的人。
-- status：0 正常、1 作者删除、2 管理人员删除（可恢复，审计日志可撤销）。
CREATE TABLE floor_comments (
  id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  post_id     bigint      NOT NULL,
  author_id   bigint      NOT NULL,
  reply_to_id bigint,
  body        text        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 500),
  status      smallint    NOT NULL DEFAULT 0 CHECK (status IN (0, 1, 2)),
  created_at  timestamptz NOT NULL DEFAULT now()
);
-- 每楼前几条评论与「展开其余」翻页：按 (post_id, id) 顺序只扫未删除的。
CREATE INDEX floor_comments_post ON floor_comments (post_id, id) WHERE status = 0;

-- 举报（楼层或评论）。同一人对同一内容只记一次；处理队列按板块（举报时内容所在板块）。
CREATE TABLE reports (
  id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  target_kind smallint    NOT NULL CHECK (target_kind IN (1, 2)),  -- 1 楼层、2 评论
  target_id   bigint      NOT NULL,
  forum_id    int         NOT NULL REFERENCES forums (id),
  reporter_id bigint      NOT NULL REFERENCES users (id),
  reason      text        NOT NULL DEFAULT '' CHECK (char_length(reason) <= 500),
  created_at  timestamptz NOT NULL DEFAULT now(),
  UNIQUE (target_kind, target_id, reporter_id)
);

-- 新的通知类型：合并的点赞通知、楼层评论（含回复评论）。上线前通知表为空，直接重建约束。
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
  CHECK (kind IN ('reply', 'mention', 'message', 'order', 'offer', 'system', 'like', 'comment'));

-- 主题列表的参与者头像：楼主之外最近发言的最多 4 个人（新的在前，不重复），在回帖分配楼层的同一条 UPDATE 里维护，
-- 列表按主键取主题时一并带出，不另查楼层。楼层被删除后发言人仍留在这里，直到被更新的发言人挤出。
ALTER TABLE topics ADD COLUMN recent_posters bigint[] NOT NULL DEFAULT '{}';
UPDATE topics t SET recent_posters = ARRAY(
  SELECT a.author_id
  FROM (
    SELECT p.author_id, max(p.floor) AS last
    FROM posts p
    WHERE p.topic_id = t.id AND p.author_id <> t.author_id
    GROUP BY p.author_id
  ) a
  ORDER BY a.last DESC
  LIMIT 4
)
WHERE t.last_floor > 1;
