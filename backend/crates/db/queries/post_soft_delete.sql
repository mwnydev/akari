-- 软删除：正常的楼层；管理人员（$2 = 2）也可以删除隐藏的楼层（隐藏理由保留，恢复后仍是隐藏）。
-- $1 id, $2 status
UPDATE posts SET status = $2 WHERE id = $1 AND (status = 0 OR (status = 3 AND $2 = 2::smallint))
