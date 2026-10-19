-- 举报处理队列、楼层隐藏、热门排序（plan 第 4、9、12 节）。各程序账号的授权见 deploy/ansible 的 grants.sql.j2
-- （按服务列出全部表的权限矩阵，backend/crates/db/tests/grants.rs 逐条核对各服务的查询）。

-- 举报按内容归成「案件」：同一内容未处理的举报都进同一件，处理一次全部结案（通知每位举报人）；
-- 结案后再有别人举报同一内容就开新的一件。同一人对同一内容仍只记一次（reports 的唯一约束）。
-- status：0 待处理、1 不违规（驳回）、2 已处理（action 记采取的措施：hide / delete / sanction）。
CREATE TABLE report_cases (
  id                bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  target_kind       smallint    NOT NULL CHECK (target_kind IN (1, 2)),  -- 1 楼层、2 评论，同 reports
  target_id         bigint      NOT NULL,
  forum_id          int         NOT NULL REFERENCES forums (id),          -- 最近一次举报时内容所在的板块
  target_author_id  bigint      NOT NULL,                                 -- 队列不给内容作者本人看
  reports           int         NOT NULL DEFAULT 1 CHECK (reports > 0),
  last_reported_at  timestamptz NOT NULL DEFAULT now(),
  created_at        timestamptz NOT NULL DEFAULT now(),
  status            smallint    NOT NULL DEFAULT 0 CHECK (status IN (0, 1, 2)),
  action            text        CHECK (action IN ('hide', 'delete', 'sanction')),
  resolved_by       bigint      REFERENCES users (id),
  resolved_at       timestamptz,
  CHECK ((status = 0) = (resolved_at IS NULL)),
  CHECK ((resolved_at IS NULL) = (resolved_by IS NULL)),
  CHECK ((status = 2) = (action IS NOT NULL))
);
CREATE UNIQUE INDEX report_cases_open_target ON report_cases (target_kind, target_id) WHERE status = 0;
-- 队列按板块逐个取最新的（板块只有几十个，站长 / 管理员也按全部板块逐个取再合并），待处理与已处理各一个部分索引。
CREATE INDEX report_cases_open ON report_cases (forum_id, id DESC) WHERE status = 0;
CREATE INDEX report_cases_resolved ON report_cases (forum_id, id DESC) WHERE status <> 0;

ALTER TABLE reports ADD COLUMN case_id bigint REFERENCES report_cases (id);
INSERT INTO report_cases (target_kind, target_id, forum_id, target_author_id, reports, last_reported_at, created_at)
SELECT r.target_kind, r.target_id, (array_agg(r.forum_id ORDER BY r.id DESC))[1],
       coalesce(max(p.author_id), max(c.author_id), 0), count(*), max(r.created_at), min(r.created_at)
FROM reports r
LEFT JOIN posts p ON r.target_kind = 1 AND p.id = r.target_id
LEFT JOIN floor_comments c ON r.target_kind = 2 AND c.id = r.target_id
GROUP BY r.target_kind, r.target_id;
UPDATE reports r SET case_id = c.id
FROM report_cases c WHERE c.target_kind = r.target_kind AND c.target_id = r.target_id;
ALTER TABLE reports ALTER COLUMN case_id SET NOT NULL;
-- 案件里最近的几条举报、结案时通知每位举报人。
CREATE INDEX reports_case ON reports (case_id, id DESC);

-- 楼层隐藏：posts.status = 3（0 正常、1 作者删除、2 管理人员删除）。点赞、评论、举报、编辑等写入语句都要求
-- status = 0，所以隐藏楼层与隐藏主题一样不能再互动；管理人员看得到内容（带标记），作者看得到内容与隐藏理由，
-- 其他人只看到「已隐藏」。当前的隐藏理由放在这张窄表里（取消隐藏时删除；历史在审计日志），不加宽 posts。
CREATE TABLE post_hides (
  post_id    bigint      PRIMARY KEY,
  reason     text        NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 500),
  hidden_by  bigint      NOT NULL REFERENCES users (id),
  hidden_at  timestamptz NOT NULL DEFAULT now()
);

-- 热门排序（plan 第 9 节「热门榜刷新间隔 5 分钟」）：forum-worker 每 5 分钟按近 14 天内新建、仍有回复的主题重算一遍，
-- 每个板块只留前 1000 个。分数随时间衰减，必须定时重算，存成 topics 的列就要每 5 分钟改写所有活跃主题的宽行
-- （且点赞要改 topics 行，失去 post_counters 窄表的意义）；单独一张窄表，整表在一个事务里替换，读的人看到的
-- 总是完整的上一版排名。列表取到 ID 后与最新列表一样按主键取主题、按当前状态（隐藏、置顶、移走）过滤。
CREATE TABLE hot_topics (
  topic_id bigint PRIMARY KEY,
  forum_id int    NOT NULL,
  score    float8 NOT NULL
);
CREATE INDEX hot_topics_forum ON hot_topics (forum_id, score DESC, topic_id DESC);
CREATE INDEX hot_topics_all ON hot_topics (score DESC, topic_id DESC);
