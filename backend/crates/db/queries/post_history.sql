-- 编辑历史，新的在前，游标分页。$1 post_id, $2 before_id, $3 limit
SELECT id, editor_id, title, html, edited_at FROM post_edits
WHERE post_id = $1 AND id < $2
ORDER BY id DESC LIMIT $3
