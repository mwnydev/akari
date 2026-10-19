CREATE TABLE forums (
  id          int         GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  parent_id   int         REFERENCES forums (id),
  slug        text        NOT NULL UNIQUE,
  name        text        NOT NULL,
  description text        NOT NULL DEFAULT '',
  position    int         NOT NULL DEFAULT 0,
  created_at  timestamptz NOT NULL DEFAULT now()
);

-- 以下三张表与 plan 第 8 节 DDL 一致。
CREATE TABLE topics (
  id             bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  forum_id       int         NOT NULL,
  author_id      bigint      NOT NULL,
  title          text        NOT NULL,
  kind           smallint    NOT NULL DEFAULT 0,  -- 普通 / 交易 / 争议
  flags          smallint    NOT NULL DEFAULT 0,  -- 1=置顶、2=精华、4=锁定、8=隐藏
  last_floor     int         NOT NULL DEFAULT 1,  -- 最大楼层 = 回复数 + 1
  last_post_id   bigint,
  last_poster_id bigint,
  last_post_at   timestamptz NOT NULL,
  created_at     timestamptz NOT NULL DEFAULT now()
) WITH (fillfactor = 80);
CREATE INDEX topics_forum_list ON topics (forum_id, last_post_at DESC, id DESC) WHERE (flags & 9) = 0;
CREATE INDEX topics_forum_pinned ON topics (forum_id, id DESC) WHERE (flags & 1) != 0 AND (flags & 8) = 0;

CREATE TABLE posts (
  id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  topic_id   bigint      NOT NULL,
  floor      int         NOT NULL,
  author_id  bigint      NOT NULL,
  html       text        COMPRESSION lz4 NOT NULL,  -- 发帖时已过滤渲染
  plain_text text        NOT NULL,                  -- 供全文搜索
  status     smallint    NOT NULL DEFAULT 0,        -- 删除只改状态，楼层保留
  created_at timestamptz NOT NULL DEFAULT now(),
  edited_at  timestamptz,
  UNIQUE (topic_id, floor)
);
CREATE INDEX posts_author ON posts (author_id, id DESC);
CREATE INDEX posts_search ON posts USING pgroonga (plain_text);
CREATE TABLE post_sources (post_id bigint PRIMARY KEY, source jsonb NOT NULL);  -- 编辑器原稿
