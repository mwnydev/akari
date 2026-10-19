#!/usr/bin/env python3
"""压测数据生成器（plan 第 12 节：按 2000 万帖子的设计容量压测）。

确定性：同样的 --posts 与 --seed 总是生成同样的数据（每个用户 / 主题 / 楼层各用自己的种子），与分片数无关。
输出 PostgreSQL COPY 文本格式，由 load.sh 经 `psql -c "COPY … FROM STDIN"` 灌入（不经过 INSERT）。

  datagen.py plan  --posts N --out DIR        生成主题与楼层元数据（主题表、楼层 ID 顺序）到 DIR
  datagen.py users --posts N                  users 表（stdout）
  datagen.py levels --posts N                 user_levels 表
  datagen.py topics --out DIR                 topics 表（读 plan 的结果）
  datagen.py listings --out DIR               trade_listings 表
  datagen.py posts --out DIR --shard i/n      posts 表的第 i 片（共 n 片，按楼层 ID 连续切分）
  datagen.py sources|counters|likes|comments --out DIR --shard i/n   同一片楼层的原稿、计数、点赞、评论

楼层 ID 按发帖时间全局递增（各主题的楼层在堆表里交错分布，与真实论坛一样），而不是一个主题的楼层挨在一起。
"""

import argparse
import array
import datetime
import hashlib
import json
import math
import os
import random
import sys

NOW = 1_792_454_400  # 2026-10-20 00:00:00 UTC
SPAN = 2 * 365 * 86_400  # 主题创建时间分布在最近两年
POSTS_PER_TOPIC = 9.64  # 平均每主题楼层数（含 1 楼与热帖，按生成结果标定）
POSTS_PER_USER = 20
HOT_TOPIC_EVERY = 20_000  # 每这么多主题有一个楼层上万的热帖（深分页）

# (slug, 名称, 主题占比, view_min_level, allow_trade)；`lounge` 只有 3 级以上能看，用来验证搜索不泄露。
FORUMS = [
    ("announcements", "站务公告", 0.2, 0, False),
    ("general", "综合讨论", 18, 0, False),
    ("tech", "技术", 8, 0, False),
    ("help", "帮助", 3, 0, False),
    ("lounge", "会员茶馆", 4, 3, False),
    ("trade", "交易区", 14, 0, True),
    ("accounts", "账号交易", 12, 0, True),
    ("giftcards", "礼品卡", 9, 0, True),
    ("games", "游戏", 10, 0, True),
    ("software", "软件与订阅", 7, 0, True),
    ("crypto", "币圈", 6, 0, False),
    ("life", "生活", 5, 0, False),
    ("jobs", "招聘求职", 2, 0, False),
    ("feedback", "意见反馈", 1, 0, False),
    ("disputes", "争议公示", 0.8, 0, False),
    ("archive", "旧帖存档", 2, 0, False),
]

# 词表：常用词在前（齐普夫分布取词），交易论坛的领域词与中英混排的商品名混在其中。
WORDS = """
的 了 是 我 你 在 有 这个 一下 可以 没有 什么 我们 他们 就是 还是 因为 所以 但是 如果 已经 现在 今天 昨天 明天
大家 自己 知道 觉得 感觉 问题 时候 东西 朋友 一个 一些 很多 非常 比较 应该 需要 希望 谢谢 请问 楼主 支持 顶 收藏
出售 收购 求购 担保 交易 价格 便宜 包邮 秒发 自动发货 手动发货 库存 售后 质保 退款 发货 确认收货 好评 差评 信誉
USDT 钱包 地址 转账 手续费 汇率 人民币 美元 链上 区块 确认 合约 签名 授权 提币 充值 交易所 OKX Arbitrum 主网 测试网
账号 会员 订阅 激活码 兑换码 礼品卡 点卡 充值卡 话费 流量 苹果ID 谷歌账号 Netflix Spotify ChatGPT Claude Steam
Switch PlayStation Xbox 原神 王者荣耀 英雄联盟 绝地求生 皮肤 装备 金币 代练 陪玩 上分 首充 月卡 季卡 年卡 永久
服务器 域名 主机 VPS 带宽 节点 线路 延迟 稳定 测速 机房 香港 日本 新加坡 美国 德国 独享 共享 优化 回程 晚高峰
教程 分享 经验 攻略 推荐 评测 对比 体验 心得 总结 记录 新手 入门 进阶 技巧 注意 避坑 骗子 曝光 举报 维权 仲裁
Rust Python Linux Docker PostgreSQL Redis Nginx 编程 开发 前端 后端 数据库 部署 运维 脚本 接口 性能 缓存 索引 查询
手机 电脑 笔记本 显卡 耳机 键盘 显示器 路由器 iPhone 安卓 小米 华为 三星 二手 全新 未拆封 国行 港版 美版 保修 发票
工作 生活 吃饭 睡觉 周末 假期 旅游 天气 心情 开心 难过 无聊 加油 哈哈 厉害 牛逼 学习 考试 面试 工资 房租 搬家
卖家 买家 下单 付款 收款 联系 私信 回复 留言 帖子 板块 版主 管理员 规则 公告 活动 抽奖 福利 优惠 折扣 限时 名额
""".split()

NAME_CHARS = "云风月星辰海山林雨雪光明晨夕安宁思远子轩浩然梓涵一诺欣怡雅静天宇志强俊杰晓峰小白阿猫老王大鱼"
PUNCT = "，，，。。！？、"


def zipf_weights(n, s=0.85):
    return [1.0 / (i + 1) ** s for i in range(n)]


CUM_WEIGHTS = []
_acc = 0.0
for _w in zipf_weights(len(WORDS)):
    _acc += _w
    CUM_WEIGHTS.append(_acc)


def scale(posts):
    topics = max(round(posts / POSTS_PER_TOPIC), len(FORUMS) * 4)
    users = max(posts // POSTS_PER_USER, 100)
    return topics, users


def rng(kind, n, seed):
    """每个实体自己的随机数生成器：与生成顺序、分片无关。"""
    h = hashlib.blake2b(f"{seed}:{kind}:{n}".encode(), digest_size=8).digest()
    return random.Random(int.from_bytes(h, "little"))


def sentence(r, n):
    """`n` 个词连成的一句，偶尔夹标点、数字，英文词后加空格。"""
    out = []
    for i, w in enumerate(r.choices(WORDS, cum_weights=CUM_WEIGHTS, k=n)):
        out.append(w)
        if i + 1 < n and r.random() < 0.12:
            out.append(r.choice(PUNCT))
        elif w.isascii() or (i + 1 < n and r.random() < 0.08):
            out.append(" ")
    if r.random() < 0.15:
        out.append(str(r.randint(1, 9999)))
    out.append(r.choice("。！？"))
    return "".join(out).replace(" ，", "，").replace(" 。", "。")


def body(r, floor):
    """正文段落。1 楼长（中位约 300 字），回帖多为一两句、带长尾（中位约 40 字）。"""
    if floor == 1:
        chars = int(min(8000, r.lognormvariate(5.7, 0.8)))
    else:
        chars = int(min(4000, r.lognormvariate(3.7, 1.0)))
    paras, n = [], 0
    while n < max(chars, 4):
        p = "".join(sentence(r, r.randint(3, 14)) for _ in range(r.randint(1, 4)))
        paras.append(p)
        n += len(p)
    return paras


def author(r, users):
    """发帖量偏斜：少数活跃用户发了大部分帖子。"""
    return 1 + min(users - 1, int(users * r.random() ** 2.5))


def esc(s):
    return s.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def ts(sec):
    """COPY 的 timestamptz 文本（UTC）。"""
    return datetime.datetime.fromtimestamp(sec, datetime.UTC).strftime("%Y-%m-%d %H:%M:%S+00")


def html_escape(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def pg_array(xs):
    return "{" + ",".join(str(x) for x in xs) + "}"


# ---------- plan：主题元数据与楼层 ID 顺序 ----------


def forum_of(r):
    total = sum(f[2] for f in FORUMS)
    x = r.random() * total
    for i, f in enumerate(FORUMS):
        x -= f[2]
        if x <= 0:
            return i + 1
    return len(FORUMS)


def topic_meta(t, topics, users, seed):
    """主题 t（从 1 开始）：板块、作者、创建时间、楼层数、各楼时间偏移、标志位。"""
    r = rng("topic", t, seed)
    forum = forum_of(r)
    created = NOW - SPAN + int(SPAN * (t - 1) / topics) + r.randint(0, 600)
    if t <= max(1, topics // HOT_TOPIC_EVERY):
        n = r.randint(5_000, 20_000) if topics >= HOT_TOPIC_EVERY else r.randint(200, 500)
    else:
        # 帕累托长尾：中位 4 楼、均值约 9 楼、1% 超过 80 楼
        n = 1 + min(3_000, int(r.paretovariate(1.4) * 3) - 3 + r.randint(0, 3))
    # 回帖间隔：平均 20 分钟到几小时，最晚到现在
    gap = r.lognormvariate(7.5, 1.2)
    horizon = max(60, min(NOW - 60 - created, int(gap * n)))
    offsets = sorted(r.randint(1, horizon) for _ in range(n - 1))
    flags = 0
    x = r.random()
    if x < 0.01:
        flags |= 8  # 隐藏
    elif x < 0.03:
        flags |= 2  # 精华
    if r.random() < 0.01:
        flags |= 4  # 锁定
    if t % max(1, topics // 40) == 0 and not flags & 8:
        flags |= 1  # 置顶（每板块约两三个）
    trade = FORUMS[forum - 1][4] and r.random() < 0.6
    return {
        "forum": forum,
        "author": author(r, users),
        "created": created,
        "n": n,
        "offsets": offsets,
        "flags": flags,
        "kind": 1 if trade else 0,
        "views": int(r.paretovariate(1.1) * 30) * (1 + n // 10),
    }


def cmd_plan(a):
    topics, users = scale(a.posts)
    os.makedirs(a.out, exist_ok=True)
    keys = array.array("q")
    first_author = array.array("i", [0])  # 下标 = 主题 ID
    # 排序键 = 发帖时间（秒）<< 24 | 主题 ID（< 2^24）；同一主题内按出现顺序还原楼层号
    for t in range(1, topics + 1):
        m = topic_meta(t, topics, users, a.seed)
        first_author.append(m["author"])
        keys.append((m["created"] << 24) | t)
        for off in m["offsets"]:
            keys.append(((m["created"] + off) << 24) | t)
    order = sorted(keys)
    del keys
    # 按时间顺序给楼层编号：同一主题内按出现顺序 = 楼层号
    floors_seen = array.array("i", bytes(4 * (topics + 1)))
    topic_of = array.array("i")
    floor_of = array.array("i")
    time_of = array.array("i")
    for k in order:
        t = k & 0xFFFFFF
        floors_seen[t] += 1
        topic_of.append(t)
        floor_of.append(floors_seen[t])
        time_of.append(k >> 24)
    del order
    with open(os.path.join(a.out, "topic_author.bin"), "wb") as f:
        first_author.tofile(f)
    with open(os.path.join(a.out, "post_topic.bin"), "wb") as f:
        topic_of.tofile(f)
    with open(os.path.join(a.out, "post_floor.bin"), "wb") as f:
        floor_of.tofile(f)
    with open(os.path.join(a.out, "post_time.bin"), "wb") as f:
        time_of.tofile(f)
    meta = {"posts": len(topic_of), "topics": topics, "users": users, "seed": a.seed}
    with open(os.path.join(a.out, "meta.json"), "w") as f:
        json.dump(meta, f)
    print(json.dumps(meta), file=sys.stderr)


def load_plan(out):
    with open(os.path.join(out, "meta.json")) as f:
        meta = json.load(f)
    arrs = {}
    for name in ("topic", "floor", "time"):
        a = array.array("i")
        with open(os.path.join(out, f"post_{name}.bin"), "rb") as f:
            a.fromfile(f, meta["posts"])
        arrs[name] = a
    return meta, arrs


def topic_authors(out, meta):
    a = array.array("i")
    with open(os.path.join(out, "topic_author.bin"), "rb") as f:
        a.fromfile(f, meta["topics"] + 1)
    return a


def post_author(meta, floor, pid, topic_author):
    if floor == 1:
        return topic_author
    return author(rng("pa", pid, meta["seed"]), meta["users"])


# ---------- 各表 ----------


def cmd_users(a):
    _, users = scale(a.posts)
    w = sys.stdout.write
    for u in range(1, users + 1):
        r = rng("user", u, a.seed)
        created = NOW - SPAN - 86_400 * 30 + int((SPAN + 86_400 * 20) * (u - 1) / users)
        if r.random() < 0.8:
            if r.random() < 0.5:
                name = "".join(r.choice(NAME_CHARS) for _ in range(r.randint(2, 4)))
            else:
                name = r.choice(["lucky", "moon", "neo", "kai", "sky", "zz", "xx", "pro", "dev"])
            name = f"{name}{u:x}"[:20]
            username = esc(name)
        else:
            username = "\\N"
        w(f"{u}\tu{u}@loadtest.invalid\t{username}\t{ts(created)}\n")


def cmd_levels(a):
    _, users = scale(a.posts)
    w = sys.stdout.write
    for u in range(1, users + 1):
        r = rng("level", u, a.seed)
        if r.random() < 0.6:
            level = min(20, 1 + int(r.expovariate(0.4)))
            w(f"{u}\t{level}\t{level * level * 100 + r.randint(0, 99)}\n")


def cmd_topics(a):
    meta, arrs = load_plan(a.out)
    seed, topics, users = meta["seed"], meta["topics"], meta["users"]
    w = sys.stdout.write
    rows = [topic_meta(t, topics, users, seed) for t in range(1, topics + 1)]
    metas_author = topic_authors(a.out, meta)
    last_id = array.array("q", bytes(8 * (topics + 1)))
    last_floor = array.array("i", bytes(4 * (topics + 1)))
    last_poster = array.array("i", bytes(4 * (topics + 1)))
    last_time = array.array("i", bytes(4 * (topics + 1)))
    recent = {}
    for i in range(meta["posts"]):
        pid = i + 1
        t = arrs["topic"][i]
        fl = arrs["floor"][i]
        au = post_author(meta, fl, pid, metas_author[t])
        last_id[t], last_floor[t], last_poster[t] = pid, fl, au
        last_time[t] = arrs["time"][i]
        if fl > 1 and au != metas_author[t]:
            lst = recent.setdefault(t, [])
            if au in lst:
                lst.remove(au)
            lst.insert(0, au)
            del lst[4:]
    for t, m in enumerate(rows, start=1):
        r = rng("title", t, seed)
        title = sentence(r, r.randint(3, 9)).rstrip("。！？")[:60]
        if m["kind"] == 1:
            title = r.choice(["【出售】", "【自动发货】", "【担保】", "[出]", "【收】"]) + title
        w(
            f"{t}\t{m['forum']}\t{m['author']}\t{esc(title)}\t{m['kind']}\t{m['flags']}\t{last_floor[t]}\t"
            f"{last_id[t]}\t{last_poster[t]}\t{ts(last_time[t])}\t{ts(m['created'])}\t{m['views']}\t"
            f"{pg_array(recent.get(t, []))}\n"
        )


def cmd_listings(a):
    meta, _ = load_plan(a.out)
    seed, topics, users = meta["seed"], meta["topics"], meta["users"]
    w = sys.stdout.write
    lid = 0
    for t in range(1, topics + 1):
        m = topic_meta(t, topics, users, seed)
        if m["kind"] != 1:
            continue
        lid += 1
        r = rng("listing", t, seed)
        price = r.choice([1, 2, 5, 9, 10, 15, 20, 30, 45, 50, 88, 99, 120, 199]) * 1_000_000
        terms = {
            "instant": r.random() < 0.4,
            "ship_window": r.choice([3600, 86_400, 259_200]),
            "confirm_window": 259_200,
            "fee_payer": r.randint(0, 2),
            "bond": "0",
            "late_slash_bps": 0,
        }
        h = hashlib.sha256(f"{seed}:listing:{t}".encode()).hexdigest()
        cny = r.choice(["\\N", "72000", "71500", "73000"])
        w(
            f"{lid}\t{t}\t{m['author']}\t421614\t{price}\t{json.dumps(terms)}\t\\\\x{h}\t"
            f"{ts(m['created'])}\t{cny}\n"
        )


def shard_range(meta, shard):
    i, n = (int(x) for x in shard.split("/"))
    per = math.ceil(meta["posts"] / n)
    return i * per, min(meta["posts"], (i + 1) * per)


def iter_posts(a):
    meta, arrs = load_plan(a.out)
    authors = topic_authors(a.out, meta)
    lo, hi = shard_range(meta, a.shard)
    for i in range(lo, hi):
        pid = i + 1
        t, fl, tm = arrs["topic"][i], arrs["floor"][i], arrs["time"][i]
        yield meta, pid, t, fl, tm, post_author(meta, fl, pid, authors[t])


def cmd_posts(a):
    w = sys.stdout.write
    for meta, pid, t, fl, tm, au in iter_posts(a):
        r = rng("post", pid, meta["seed"])
        paras = body(r, fl)
        status = 0
        if fl > 1 and r.random() < 0.02:
            status = r.choice([1, 2])
        html = "".join(f"<p>{html_escape(p)}</p>" for p in paras)
        plain = "\n".join(paras)
        w(f"{pid}\t{t}\t{fl}\t{au}\t{esc(html)}\t{esc(plain)}\t{status}\t{ts(tm)}\n")


def cmd_sources(a):
    w = sys.stdout.write
    for meta, pid, _t, fl, _tm, _au in iter_posts(a):
        paras = body(rng("post", pid, meta["seed"]), fl)
        doc = {
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": p}]} for p in paras],
        }
        w(f"{pid}\t{esc(json.dumps(doc, ensure_ascii=False))}\n")


def social(meta, pid, fl):
    """楼层的点赞数与评论数：多数楼层没有，1 楼与少数楼层很多。"""
    r = rng("social", pid, meta["seed"])
    likes = int(r.paretovariate(1.3)) - 1 if r.random() < (0.6 if fl == 1 else 0.25) else 0
    comments = int(r.paretovariate(1.6)) if r.random() < 0.06 else 0
    return r, min(likes, meta["users"] - 1, 2000), min(comments, 200)


def cmd_counters(a):
    w = sys.stdout.write
    for meta, pid, _t, fl, _tm, _au in iter_posts(a):
        _, likes, comments = social(meta, pid, fl)
        if likes or comments:
            w(f"{pid}\t{likes}\t{comments}\n")


def cmd_likes(a):
    w = sys.stdout.write
    for meta, pid, _t, fl, tm, _au in iter_posts(a):
        r, likes, _ = social(meta, pid, fl)
        if not likes:
            continue
        seen = set()
        while len(seen) < likes:
            seen.add(author(r, meta["users"]))
        for u in seen:
            w(f"{pid}\t{u}\t{ts(min(NOW, tm + r.randint(1, 86_400)))}\n")


def cmd_comments(a):
    w = sys.stdout.write
    # 评论 ID：每楼最多 200 条，按楼层 ID 预留区间，分片之间不冲突（ID 不连续无妨）
    for meta, pid, _t, fl, tm, _au in iter_posts(a):
        r, _, comments = social(meta, pid, fl)
        for k in range(comments):
            cid = pid * 200 + k
            cau = author(r, meta["users"])
            text = sentence(r, r.randint(2, 10))[:500]
            reply = author(r, meta["users"]) if k and r.random() < 0.3 else "\\N"
            w(f"{cid}\t{pid}\t{cau}\t{reply}\t{esc(text)}\t0\t{ts(min(NOW, tm + 60 * (k + 1)))}\n")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("cmd")
    p.add_argument("--posts", type=int, default=2_000_000)
    p.add_argument("--seed", type=int, default=20261020)
    p.add_argument("--out", default=".run/loadtest-data")
    p.add_argument("--shard", default="0/1")
    a = p.parse_args()
    cmds = {
        "plan": cmd_plan,
        "users": cmd_users,
        "levels": cmd_levels,
        "topics": cmd_topics,
        "listings": cmd_listings,
        "posts": cmd_posts,
        "sources": cmd_sources,
        "counters": cmd_counters,
        "likes": cmd_likes,
        "comments": cmd_comments,
    }
    if a.cmd not in cmds:
        p.error(f"unknown command {a.cmd}")
    sys.stdout.reconfigure(newline="\n")
    cmds[a.cmd](a)


if __name__ == "__main__":
    main()
