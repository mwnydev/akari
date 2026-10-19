-- 编辑前把当前版本存入历史；已删除的楼层不处理。$1 post_id, $2 editor_id, $3 old_title
INSERT INTO post_edits (post_id, editor_id, title, html, source)
SELECT p.id, $2, $3, p.html, s.source
FROM posts p JOIN post_sources s ON s.post_id = p.id
WHERE p.id = $1 AND p.status = 0
FOR UPDATE OF p
RETURNING id
