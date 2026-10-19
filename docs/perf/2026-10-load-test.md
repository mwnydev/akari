# 压测报告 2026-10：PGroonga 搜索、游客缓存、200 万帖子本机压测

阶段 1 关卡（plan 第 15 节）是「2000 万帖子在生产规格上压测达标；PGroonga 定案」。开发机只有约 15 GB 内存、与其他任务共用，
本轮在本机跑 **约 200 万楼层**，把结果按数据量外推到 2000 万；2000 万的正式压测按文末步骤在生产规格机器上做（PROGRESS.md 偏差说明）。

## 结论

| 项目 | 结论 |
| --- | --- |
| 1 倍峰值（1024 请求/秒 + 1500 条 WebSocket 长连接，5 分钟） | 全部阈值通过，错误率 0 |
| 帖子页 / 板块页 API | p95 2.3–3.4 ms、p99 2.9–4.7 ms（目标 50 / 150 ms） |
| 发帖 / 回复 / 点赞 | p95 4.0–4.9 ms（目标 100 ms） |
| 全文搜索（楼层正文 / 主题标题） | p95 53 / 6.4 ms（目标 300 ms） |
| 资源（1 倍峰值） | PostgreSQL 平均 0.73 核、峰值 1.6 核；forum-api 0.54 核；Valkey 0.18 核、内存 265 MB |
| 2 倍压力 | 本机未完成（k6 自身吃满内存），见「2 倍压力」 |
| PGroonga | **定案**：TokenNgram + NormalizerNFKC150，在 Groonga 里按 id 倒序取前 N 条再回表 |

## 环境

| 项目 | 配置 |
| --- | --- |
| 机器 | WSL2，20 线程，15.7 GB 内存（与其他任务共用），NVMe |
| PostgreSQL | `groonga/pgroonga:4.1.0-debian-18`（PostgreSQL 18 + PGroonga 4.1.0），容器限 3 GB；shared_buffers 1.5 GB、effective_cache_size 2 GB、work_mem 32 MB、random_page_cost 1.1、effective_io_concurrency 200、jit off、wal_compression zstd、default_toast_compression lz4、pg_stat_statements |
| Valkey | `valkey/valkey:9.1.2`，容器限 512 MB，maxmemory 384 MB |
| forum-api | release 构建（thin LTO），1 个实例，数据库连接池 20 |
| 压测工具 | k6 1.3.0（`grafana/k6:1.3.0` 镜像，host 网络），与被测服务在同一台机器 |

注意：最初几轮 forum-api 经 Docker 的端口映射（docker-proxy 用户态转发）连数据库和 Valkey，1500 条 WebSocket 同时在线时
Valkey 建连超时、整体雪崩（p95 > 10 s）。改为直连容器 IP 后同样负载 p95 降到 3 ms。生产是本机 Unix 套接字 / 回环直连，
不经过这一层，所以这是测试环境的问题，不是应用瓶颈；但说明 **Valkey 连接池在建连变慢时会拖住 WebSocket 心跳**，
上线前的告警里应包含 Valkey 建连耗时。

## 数据（`deploy/loadtest/datagen.py`，确定性，COPY 灌库）

| 表 | 行数 | 说明 |
| --- | --- | --- |
| users | 100,000 | 8 成设置了用户名（中文名与拼音名混合），6 成有等级 |
| forums | 16 | 含 5 个交易板块、1 个 3 级可见的板块（验证搜索不泄露） |
| topics | 207,469 | 1% 隐藏、2% 精华、1% 锁定、每板块两三个置顶；交易板块里 6 成是交易帖 |
| posts | 1,993,711 | 每主题楼层数为帕累托长尾（中位 4、1% 超过 80），另有 10 个 5000–20000 楼的热帖；楼层 ID 按全站发帖时间递增（各主题的楼层在堆表里交错） |
| post_sources | 1,993,711 | Tiptap JSON 原稿 |
| trade_listings | 63,453 | 价格、条款、参考汇率 |
| post_likes / post_counters | 149 万 / 约 40 万 | 1 楼 6 成有赞、回帖 1/4 有赞，长尾 |
| floor_comments | 26 万 | |

正文：1 楼中位约 300 字、回帖中位约 40 字，平均 137 字（plain_text 合计 686 MB）。词表约 400 个常用词与交易领域词按齐普夫分布取词，
**比真实语料的词汇少得多**，所以每个词都很常见（「出售」命中 18%，「我们」命中 42%）——搜索延迟按这份数据看是偏悲观的。

灌库耗时（6 路并行 COPY，二级索引灌完再建）：数据 55 秒；B-tree 索引合计约 7 秒；PGroonga 全文索引 25 秒；VACUUM ANALYZE 5 秒。
库总大小 5.4 GB（含 PGroonga 文件，不含 WAL）。

| 索引 | 大小 | 建索引耗时 |
| --- | --- | --- |
| posts_pkey | 72 MB | — |
| posts (topic_id, floor) 唯一 | 60 MB | 2.9 s |
| posts_author | 60 MB | 0.7 s |
| post_likes_recent / pkey | 58 / 56 MB | 0.4 s |
| topics_forum_list / topics_latest | 8.8 / 6.8 MB | 0.2 / 1.6 s |
| **posts_search**（PGroonga，TokenNgram + NFKC150，INCLUDE id） | 约 1.8 GB | 24–29 s |
| **topics_search**（PGroonga，标题，INCLUDE id、forum_id） | 约 170 MB | 包含在迁移的 29 s 里 |

PGroonga 的数据在 `base/<库 OID>/pgrn*` 文件里，`pg_relation_size` 显示为 0；删掉的 PGroonga 索引要等下一次 VACUUM 才回收文件
（本轮比较分词器时一度多占了 8 GB）。

## 结果（1 倍峰值，5 分钟）

流量组合（`deploy/loadtest/k6/forum.js`，与 SSR 渲染页面时调用的接口一致）：游客浏览 700/s、登录用户浏览 380/s（主题第 1 页 45%、
热帖与普通帖的后续页 15%、板块第 1 页 15%、板块第 2–200 页 12%、第 200 页之后的游标深分页 3%、首页最新 8%、板块目录 2%；
登录用户另有 1 成是 `/me`）、回帖 10/s、发主题 1/s、点赞 / 取消赞 10/s（一半在热帖上，推送给订阅热帖的 WebSocket）、搜索 25/s、
WebSocket 长连接 1500 条（各订阅一个热帖，60–120 秒后断开重连）。

| 接口 | 次数 | p50 ms | p95 ms | p99 ms | 最大 ms |
| --- | --- | --- | --- | --- | --- |
| 主题第 1 页 `topic` | 140,914 | 1.3 | 2.3 | 2.9 | 89.9 |
| 主题后续页 / 热帖末页 `topic_deep` | 46,830 | 1.7 | 3.1 | 4.7 | 89.9 |
| 板块第 1 页 `board` | 46,866 | 0.9 | 3.4 | 4.0 | 87.0 |
| 板块第 2–200 页 `board_deep` | 37,415 | 2.3 | 3.2 | 3.8 | 88.4 |
| 板块游标深分页 `board_cursor` | 9,314 | 2.0 | 2.7 | 3.2 | 39.2 |
| 首页最新 `latest` | 24,984 | 1.1 | 3.9 | 11.2 | 89.9 |
| 板块目录 `forums` | 6,272 | 0.5 | 1.2 | 1.6 | 30.3 |
| `/me` | 11,407 | 0.7 | 1.2 | 1.6 | 60.5 |
| 回帖 `reply` | 3,001 | 3.7 | 4.9 | 6.6 | 93.8 |
| 发主题 `new_topic` | 301 | 3.5 | 4.8 | 5.8 | 89.7 |
| 点赞 `like` | 3,001 | 2.9 | 4.0 | 4.7 | 51.2 |
| 搜索楼层 `search` | 5,299 | 31.3 | 53.2 | 56.0 | 133.9 |
| 搜索标题 `search_topic` | 2,202 | 4.5 | 6.4 | 7.4 | 14.1 |
| WebSocket 连上到收到 ready | 5,550 | 1.0 | 2.0 | 3.0 | 83.0 |
| 游客浏览合计（走游客缓存） | 210,001 | 0.8 | 2.9 | 3.9 | 89.9 |
| 登录用户浏览合计（每次回源） | 114,001 | 1.9 | 3.3 | 4.0 | 89.9 |

合计 337,806 个请求、1024 请求/秒、失败 0；推送 29,798 条；k6 无丢弃的迭代。最大值（约 90 ms）都出现在同一两秒内，是同机 k6 的停顿。

资源：PostgreSQL 容器峰值 162%（1.6 核）、平均 0.73 核、内存 2.1 GB；缓存命中率 98.6%。forum-api 平均 0.54 核、峰值 0.71 核、
常驻 310 MB。Valkey 平均 0.18 核，内存 265 MB（游客缓存条目 + 会话 + 有序集合），0 淘汰；读命中 97%。

重连风暴（`WS_STORM=1`，1500 条连接在同一秒发起）：ready p50 2 ms、p95 1.05 s、最慢 1.3 s，全部连上。

### 慢查询（pg_stat_statements，按总耗时）

| 总耗时 ms | 次数 | 平均 ms | 最大 ms | 语句 |
| --- | --- | --- | --- | --- |
| 158,380 | 7,501 | 21.1 | 82.0 | 搜索第一步 `pgroonga_command('select', …)`（楼层与标题合计） |
| 14,245 | 92,873 | 0.15 | 2.6 | 按主键批量取主题（列表、主题页） |
| 5,098 | 103,100 | 0.05 | 0.8 | 楼层点赞 / 评论计数 |
| 4,952 | 24,546 | 0.20 | 5.2 | 主题第 N 页（楼层区间） |
| 3,046 | 3,001 | 1.01 | 18.3 | 回帖（分配楼层的多 CTE 语句） |
| 1,440 | 4,833 | 0.30 | 1.0 | 搜索第二步回表 |

除搜索外没有超过 50 ms 的语句。搜索占数据库总耗时的大头：每次 20 ms 左右，几乎全是 Groonga 在倒排索引里取命中、按 id 排序。

## 找到并修掉的瓶颈

1. **搜索的执行计划会选错**（EXPLAIN 证据）。直接写 `WHERE plain_text &@~ q ORDER BY id DESC LIMIT 20` 时，PGroonga 的命中数估计是
   固定的 0.1%，规划器总是选「主键倒序扫描 + 逐行匹配」：常见词 40 ms，少见短语「回程 晚高峰」（257 条）要扫 66 万行、**4.6–5.4 s**；
   带上主题与板块的连接时规划器还会选「每个主题跑一次 PGroonga 位图扫描」的嵌套循环，**跑了几分钟、取消不掉**，只能终止后端进程。
   PostgreSQL 18 + PGroonga 4.0.4 起支持「有序索引扫描」，但只在排序列前面的索引列是等值条件时生效（试了 `(plain_text, id)`、
   `(id, plain_text)`、正序倒序都还是全量取出再排序）。
   改法：索引 `INCLUDE (id)`，在 Groonga 里用 `pgroonga_command('select', …, 'sort_keys', '-id', 'limit', N)` 取前 N 个 id（不回表），
   再按主键回 PostgreSQL 取行并检查可见性（未删除、主题未隐藏、板块对访问者可见），凑不满一页就在 id 之前再取一轮（后一轮更多）。
   常见词 40–90 ms → 20 ms 左右，少见短语 5 s → 10 ms；`search_posts_rows.sql` 回表只走 posts_pkey / topics_pkey（EXPLAIN 测试）。
2. **游客缓存**（下节）：同样的浏览接口，游客（走缓存）p50 0.8 ms，登录用户（每次回源，另加读会话与账号）p50 1.9 ms。
3. **搜索占用连接池**：搜索在事务里（`SET LOCAL statement_timeout = '3s'`）占一个数据库连接，每进程并发上限从 16 降到 6
   （连接池 20），保证列表、主题页、发帖不被搜索饿死。
4. 测试环境：docker-proxy 端口转发在 1500 条 WebSocket 下成为瓶颈（见「环境」），改为直连。

还可以做、但这轮没做的：每个请求的名片 `HGETALL` 是流水线批量的，但 5 分钟里调用了 1436 万次（每页约 25 个用户），是 Valkey
的主要负载；用户名片可以改成一个 `HMGET`-风格的 Lua 或进程内短 TTL 缓存。等真实流量下再看。

## 搜索设计

- 接口 `GET /api/v1/search?q=&type=post|topic|user&board=&cursor=`，游客可用。按 id 倒序（新的在前），游标 `{扫描到的 id}.{已返回条数}`，
  一个查询最多翻到 1000 条（plan 第 8 节）。`type=user` 走已有的用户名前缀索引。
- 查询串：按空白切成最多 5 个词、最多 64 字，**每个词加双引号作为短语**，词之间是「与」；用户写不出 `OR`、`-`、括号、`列名:` 等运算符，
  也就没有 Groonga 查询注入（单元测试 + 数据库测试验证 `OR`、`-好评` 都只当普通文字）。过滤条件只由服务器用整数拼出。
- 高亮：服务器把片段切成 `{text, hit}` 数组（第一处命中前留 40 字，共 160 字），前端当纯文本渲染，**不拼 HTML**，没有 XSS 面；
  正文来源本来就是过滤后的纯文本列。
- 可见性：楼层未删除、主题未隐藏、板块在访问者等级（或管理范围）内；`board=` 指定看不到的板块返回 404。
  已编辑楼层的旧版本在 VACUUM 前也在索引里：id 去重，回表按当前行判断，片段也只取当前正文。
- 限流：游客按 IP 每分钟 20 次、登录用户每分钟 60 次（滑动窗口）；每进程最多 6 个搜索并发，排队超过 2 秒返回 429；单条语句 3 秒超时。
  服务端渲染的搜索页把 Cloudflare 的 `CF-Connecting-IP` 转给后端（后端只在连接来自本机时信任它），否则所有游客共用一个名额。
- 前端：`/search` 页（帖子 / 主题 / 用户三栏、板块筛选、更早的结果、不进搜索引擎索引）；顶栏搜索是原生 popover 里的普通 GET 表单
  （无 JS 也能用），输入两个字以上停顿 300 ms 列出标题匹配的主题（取消过期请求）。

## PGroonga 定案

**采用 PGroonga**，分词 `TokenNgram("unify_alphabet", false, "unify_digit", false, "unify_symbol", false)`，规范化 `NormalizerNFKC150`。

| 分词 / 规范化（200 万楼层） | 建索引 | 大小 | 「出售」36.9 万条 | 「我们」83.2 万条 | 「回程 晚高峰」257 条 | 「gpt」搜「ChatGPT」 |
| --- | --- | --- | --- | --- | --- | --- |
| TokenBigram + NormalizerAuto（原默认） | 31 s | 1650 MB | 62 ms | 69 ms | 23 ms | **0 条**（连续字母当一个词） |
| TokenNgram（字母数字也二元切分）+ NFKC150 | 24 s | 1814 MB | 52 ms | 67 ms | 19 ms | 21.5 万条 |
| TokenBigramSplitSymbolAlphaDigit + NFKC150 | 28 s | 1810 MB | 56 ms | 67 ms | 18 ms | 21.5 万条 |

（Groonga 里取前 20 条的耗时，单个请求、无并发。）

理由：中文不需要词典、新帖写入即可搜到；字母按 N 元组切分后「gpt」「flix」这类词中间也能命中（交易区商品名大量中英混排），
代价是索引大 10%；NFKC150 是固定版本的 Unicode 规则（全角半角、大小写统一），升级 Groonga 不会悄悄改变已有索引的切分结果。
两个 N 元组方案结果相同，TokenNgram 建得更快、选项明确，选它。

与 zhparser 的对比没有做：PostgreSQL 18 没有可用的现成 zhparser 包，需要自己编译 SCWS 与扩展，并维护词典
（商品名、游戏名、新词都要进词典才搜得到）；zhparser 基于 tsvector + GIN，同样存在「命中多时按时间排序要取全部命中」的问题，
而且 GIN 没有 Groonga 这种在索引内排序取前 N 的办法。PGroonga 的已知代价：
- 延迟与命中数成正比（取前 N 也要遍历全部命中）：200 万楼层、命中 83 万时约 70 ms。
- 索引是正文大小的约 2.6 倍（1.8 GB / 686 MB），在 PostgreSQL 的共享缓冲之外占页缓存。
- PGroonga 的扫描中途不响应取消（见上文的嵌套循环），所以搜索 SQL 固定为两步、不交给规划器，并设语句超时与并发上限。

## 外推到 2000 万楼层

| 项目 | 200 万（实测） | 2000 万（外推） | 依据 |
| --- | --- | --- | --- |
| 库大小（不含 WAL） | 5.4 GB | 约 54 GB | 行数线性；plan 估计 40–80 GB |
| B-tree 索引 | posts 三个约 190 MB | 约 2 GB | plan「约 2 GB，可常驻内存」 |
| PGroonga | 约 2.0 GB | 约 20 GB | 线性；生产 32 GB 内存里 shared_buffers 8 GB 之外的页缓存放得下大部分 |
| 灌库 + 建索引 | 约 1.5 分钟 | 约 15–20 分钟 | COPY 与建索引都是线性 |
| 主题页 / 板块页 p95 | 2–4 ms | 预计 < 10 ms | 都是单次索引查找（楼层区间、主键批量、有序集合），B 树深度只多一层；游客缓存命中与数据量无关 |
| 发帖 / 回复 p95 | 5 ms | 预计 < 10 ms | 每个索引多一层，PGroonga 插入为增量 |
| 搜索 p95 | 53 ms | **常见词约 300–600 ms，一般查询 < 100 ms** | 与命中数成正比；本数据集每个词都很常见（悲观） |

搜索在 2000 万上可能达不到 p95 < 300 ms 的只有「单个极常见词」（命中几百万条）。真实语料词汇长尾得多，典型查询命中数是本数据的
几十分之一；正式压测时用真实语料或更大的词表重跑。若达不到，按顺序处理：(1) 只有一个词且命中超过阈值时，先只搜最近一段 id
（Groonga `filter` 里加 `id > 最新 id − 窗口`），翻页时再扩大；(2) 搜索单独一个只读副本。

### 2 倍压力

本机 2 倍压力（2050 请求/秒 + 3000 条 WebSocket）没有跑完：k6 的每个 WebSocket VU 是一个常驻的 JS 运行时，1 倍时 k6 自身已用到
10.7 GB，2 倍时被系统内存不足进程终止。1 倍下数据库 0.73 核、forum-api 0.54 核、Valkey 0.18 核，按线性外推 2 倍约
1.5 / 1.1 / 0.4 核，在 8 核生产机的 70% CPU 指标内（plan 第 12 节）。现在 `run.sh` 把 k6 容器限制在 2 GB（`K6_MEMORY`），
WebSocket 默认 300 条；2 倍压力与 3000 条长连接要在单独的压测机上跑（下节）。

## 游客缓存设计

三层：Cloudflare 边缘（整页 HTML 60 秒）→ Valkey 游客响应缓存（forum-api）→ PostgreSQL。

- **只缓存游客**：会话里没有用户（未登录、伪造或过期的 Cookie）才读写，登录用户的请求一律回源、不写入（测试：登录用户请求四个接口后
  Valkey 里没有任何游客条目；同一地址游客拿到 `can_reply: false`，登录用户拿到 `true`）。缓存的是 JSON 响应，组装函数只用游客视角。
- **条目**：板块目录、板块第 1–200 页、首页最新（第一页）、主题第 N 页（不超过末页）。任意游标的深分页与超出末页的空页不缓存（键空间无界）；
  超过 1 MB 的响应不缓存。
- **版本号精确失效**：每个条目记下写入时依赖的版本号（`forums`、`board:{id}`、`latest`、`topic:{id}`），写操作给对应版本号加一。
  读者比较版本，不一致就当不存在，**不会返回写入之前的内容**（回源期间又有写入时，写回的条目带的是旧标签，也不算命中）。
  回帖、发主题、置顶 / 精华 / 锁定 / 隐藏 / 移动、改标题 → 主题 + 所在板块 + 首页；编辑、删除、恢复、点赞、评论、撤销管理操作 → 该主题。
  后台改板块（新建、改名、改可见等级）须调用 `guest::forums_changed`（目前板块只能用 SQL 改，接口上线时接上）。
- **单飞**：未命中时只有拿到回源锁（`SET NX PX 3000`）的请求回源，其余每 10 ms 查一次、最多 200 ms，之后自己回源（不写缓存）。
  测试：冷启动 24 个并发请求只有 1 个回源。
- **过期后先用旧值**：新鲜期 30 秒；过期后版本没变时一个请求回源，其余拿旧值（最多再 90 秒）。只按时间刷新的只有名片、浏览数、交易帖库存。
- **隐藏**：回源发现主题已不存在或对游客不可见（404）时连同旧条目一起删除，旧页面不会再当作过期内容返回。
- Valkey 不可用时照常回源；条目有过期时间，生产 `maxmemory-policy volatile-lru` 下内存满了先淘汰它们（注意会话也带过期时间）。
- 边缘：`Cloudflare-CDN-Cache-Control: public, max-age=60, stale-while-revalidate=30, stale-if-error=300`（过期后台回源，只多一次回源的时间；
  源站故障时最多再用 5 分钟旧页）；带有效会话的页面 `no-store`。

## 上线配置（Cloudflare）

边缘缓存要两条缓存规则，否则登录用户会拿到边缘缓存的游客页：
1. 请求带 `sid` Cookie（`http.cookie contains "sid="`）→ 绕过缓存。
2. 其余论坛页面（非 `/api/`、非 `/_next/`）→ 可缓存，边缘 TTL 遵循源站（`Cloudflare-CDN-Cache-Control`）。

## 2000 万楼层的正式压测步骤（生产规格：8 核 32 GB NVMe）

在**单独的库与 Valkey 上**做，不用生产数据。压测机与被测机分开（k6 本身要几 GB 内存与数核）。

```bash
# 被测机：PostgreSQL 18 + PGroonga 4.1（参数按 plan 第 8 节；另开 pg_stat_statements），Valkey 9.1.2
createdb -U akari akari_lt && psql -U akari -d akari_lt -c 'CREATE EXTENSION pg_stat_statements'
cd backend && sqlx migrate run --source crates/db/migrations --database-url postgres://akari@/akari_lt

# 灌 2000 万楼层（约 15–20 分钟、需约 60 GB 空闲磁盘 + 15 GB WAL）
PSQL='psql postgres://akari@/akari_lt' OUT=/srv/lt/data JOBS=8 deploy/loadtest/load.sh 20000000
cat /srv/lt/data/index-build.tsv            # 各索引建索引耗时（含 PGroonga）

# forum-api：与生产相同的 release 构建与 systemd 配置，DATABASE_URL 指向 akari_lt，KV_PREFIX=lt，
# ALLOWED_ORIGINS 含压测用的来源；两个实例都起来（与生产一致）
psql postgres://akari@/akari_lt -qAt -f deploy/loadtest/targets.sql > /srv/lt/data/targets.json
VALKEY_URL=redis://127.0.0.1:6379 COOKIE_SECRET=<压测实例的密钥> \
  cargo run --release -p forum-api --example loadtest_sessions -- 3000 > /srv/lt/data/sessions.txt

# 压测机（把 /srv/lt/data 拷过去；BASE 指向被测机的 forum-api，经内网直连）
PSQL='psql postgres://akari@被测机/akari_lt' VALKEY_CLI='valkey-cli -h 被测机' BASE=http://被测机:8080 \
  K6_MEMORY=24g WS_CONNS=3000 DATA=/srv/lt/data STATS_PROCS='postgres|valkey-server|forum-api' \
  deploy/loadtest/run.sh prod-1x 1 15m
# 2 倍余量（plan 第 12 节）
... deploy/loadtest/run.sh prod-2x 2 10m
```

`STATS_PROCS` 需要在被测机上跑 `pidstat`（sysstat）；压测机与被测机分开时，在被测机上另开 `pidstat -h -u -r -C 'postgres|valkey-server|forum-api' 5`。
达标条件：`k6.txt` 末尾 `all thresholds passed`，CPU < 70%、内存 < 80%，`statements.txt` 里除搜索外没有平均 > 50 ms 的语句。
压测结束后删掉 `akari_lt` 库、`lt:` 前缀的 Valkey 键与压测会话（`sess:*` 里 `uid` 为压测用户的；或直接用单独的 Valkey 数据库号）。
