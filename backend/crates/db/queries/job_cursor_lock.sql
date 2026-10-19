-- 取后台任务进度并加锁：另一个 worker 正在处理时跳过（不返回行）。调用前已确保行存在。$1 name
SELECT position FROM job_cursors WHERE name = $1 FOR UPDATE SKIP LOCKED
