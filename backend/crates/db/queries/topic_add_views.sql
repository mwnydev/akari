-- 批量累加浏览数。$1 ids, $2 counts
UPDATE topics t SET view_count = t.view_count + v.n
FROM unnest($1::bigint[], $2::bigint[]) AS v(id, n)
WHERE t.id = v.id
