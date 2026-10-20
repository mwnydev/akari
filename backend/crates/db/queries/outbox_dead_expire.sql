-- 丢弃进死信超过 $1 秒的消息并清空 payload。
UPDATE outbox SET discarded_at = now(), payload = '{}'
WHERE failed_at IS NOT NULL AND discarded_at IS NULL AND failed_at < now() - make_interval(secs => $1)
