-- 设置 / 清除标志位并返回旧值。$1 id, $2 set, $3 clear
UPDATE topics t SET flags = (t.flags | $2::smallint) & ~($3::smallint)
FROM (SELECT flags FROM topics WHERE id = $1 FOR UPDATE) old
WHERE t.id = $1
RETURNING old.flags AS "old!", t.flags AS "new!", t.forum_id
