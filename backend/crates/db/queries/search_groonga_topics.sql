-- 全文搜索主题标题：同 search_groonga_posts.sql，索引 topics_search 带 forum_id，访问者可见的板块直接在 Groonga 里过滤。
-- $1 Groonga 查询, $2 过滤条件（in_values(forum_id, …) [&& id < N]）, $3 条数
SELECT pgroonga_command('select', ARRAY[
  'table', pgroonga_table_name('topics_search'),
  'match_columns', 'title',
  'query', $1,
  'filter', $2,
  'sort_keys', '-id',
  'limit', $3::int8::text,
  'output_columns', 'id'
])::jsonb AS "result!"
