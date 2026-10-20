-- 回复某一楼（Discourse 式「回复 #N」）：回帖时可指定同一主题里一个正常显示的楼层，回帖里显示「回复 #N 名字」，
-- 并通知被回复楼层的作者。与 topic_id 一样不设外键（楼层只改状态、不物理删除）；写入时在回帖语句里校验
-- 同一主题、状态正常，否则回帖接口拒绝。新列全为 NULL，加列不重写表。
ALTER TABLE posts ADD COLUMN reply_to_post_id bigint;
