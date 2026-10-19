-- 全文搜索楼层第一步：在 PGroonga 索引 posts_search 的 Groonga 表里匹配并按 id 倒序取前 N 个（不回表）。
-- 结果是 Groonga select 的 JSON：[[返回码, …], [[[命中总数], [[列名, 类型]], [id], [id], …]]]。
-- 不在 PostgreSQL 里写 `WHERE plain_text &@~ … ORDER BY id DESC LIMIT`：规划器对命中数的估计不可靠，见迁移 20261212000001。
-- 已编辑楼层的旧版本在清理前也在索引里，所以 id 可能重复、内容可能已不再命中；回表时按当前行判断可见性。
-- $1 Groonga 查询（search::groonga_query 生成，只含加引号的词组）, $2 过滤条件（'' 或 'id < N'）, $3 条数
SELECT pgroonga_command('select', ARRAY[
  'table', pgroonga_table_name('posts_search'),
  'match_columns', 'plain_text',
  'query', $1,
  'filter', $2,
  'sort_keys', '-id',
  'limit', $3::int8::text,
  'output_columns', 'id'
])::jsonb AS "result!"
