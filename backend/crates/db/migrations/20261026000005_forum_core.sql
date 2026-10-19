-- 论坛核心（plan 第 4、8、9 节）：板块权限、用户等级、管理角色、浏览数、编辑历史。

-- 板块权限：可见 / 发帖 / 回复所需最低等级。游客等级为 0，注册用户至少为 1。
ALTER TABLE forums
  ADD COLUMN view_min_level  smallint NOT NULL DEFAULT 0,
  ADD COLUMN post_min_level  smallint NOT NULL DEFAULT 1,
  ADD COLUMN reply_min_level smallint NOT NULL DEFAULT 1;
CREATE INDEX forums_parent ON forums (parent_id, position);

-- 普通论坛等级（plan 第 7 节）。没有记录的用户按 1 级、0 经验处理；经验流水表在等级功能里补。
CREATE TABLE user_levels (
  user_id    bigint      PRIMARY KEY REFERENCES users (id),
  level      smallint    NOT NULL DEFAULT 1 CHECK (level BETWEEN 1 AND 20),
  exp        bigint      NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- 管理角色（plan 第 9 节）。版主的 forum_id 为管辖板块，其余角色为空（全站）。
-- role：1=站长、2=超级管理员、3=管理员、4=版主、5=仲裁员。
CREATE TABLE staff_roles (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id    bigint      NOT NULL REFERENCES users (id),
  role       smallint    NOT NULL CHECK (role BETWEEN 1 AND 5),
  forum_id   int         REFERENCES forums (id),
  granted_by bigint      REFERENCES users (id),
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK ((role = 4) = (forum_id IS NOT NULL))
);
CREATE UNIQUE INDEX staff_roles_unique ON staff_roles (user_id, role, coalesce(forum_id, 0));

-- 浏览数先记 Valkey，worker 每 10 秒批量回写（plan 第 8 节）。
ALTER TABLE topics ADD COLUMN view_count bigint NOT NULL DEFAULT 0;

-- 编辑历史：每次编辑保存编辑前的版本，只追加。
CREATE TABLE post_edits (
  id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  post_id     bigint      NOT NULL,
  editor_id   bigint      NOT NULL,
  title       text,                    -- 1 楼编辑时同时保存旧标题
  html        text        COMPRESSION lz4 NOT NULL,
  source      jsonb       NOT NULL,
  edited_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX post_edits_post ON post_edits (post_id, id DESC);
