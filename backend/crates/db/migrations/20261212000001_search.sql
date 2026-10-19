-- 全文搜索（plan 第 8 节「搜索」，选型与压测数据见 docs/perf/2026-10-load-test.md）。
--
-- 分词：TokenNgram 二元切分（中文不需要词典，新帖写入即可搜到），字母与数字也按二元切分
-- （unify_alphabet / unify_digit 为 false），「gpt」能搜到「ChatGPT」；默认的 TokenBigram 把连续字母当一个词，搜不到词中间。
-- 规范化：NormalizerNFKC150（全角半角统一、大小写不敏感），与默认 NormalizerAuto 相比是固定版本的 Unicode 规则，
-- 升级 Groonga 不会悄悄改变已有索引的切分结果。
--
-- INCLUDE 的列不进倒排索引，只存进 Groonga 的表：搜索在 Groonga 里按 id 倒序取前 N 条（不回表），
-- 再按主键回 PostgreSQL 取行并检查可见性（queries/search_*.sql）。规划器对 PGroonga 的命中数估计不可靠，
-- 直接写 WHERE … &@~ … ORDER BY id DESC LIMIT 会在常见词上走主键倒序扫描逐行匹配、在罕见词上扫完全表。
DROP INDEX posts_search;
CREATE INDEX posts_search ON posts USING pgroonga (plain_text) INCLUDE (id)
  WITH (tokenizer = 'TokenNgram("unify_alphabet", false, "unify_digit", false, "unify_symbol", false)',
        normalizers = 'NormalizerNFKC150');

CREATE INDEX topics_search ON topics USING pgroonga (title) INCLUDE (id, forum_id)
  WITH (tokenizer = 'TokenNgram("unify_alphabet", false, "unify_digit", false, "unify_symbol", false)',
        normalizers = 'NormalizerNFKC150');
