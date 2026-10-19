//! PGroonga 搜索（迁移 20261212000001 的分词与规范化、Groonga 侧按 id 倒序取前 N 个）。需要 PostgreSQL（`DATABASE_URL`）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_db::PgPool;
use forum_db::search::{self, groonga_query};

/// 两个板块各两个主题；楼层正文为 `texts`，依次挂在主题 1、2、3、4 上（楼层号递增）。返回楼层 id。
async fn seed(db: &PgPool, texts: &[&str]) -> Vec<i64> {
    sqlx::raw_sql(
        "INSERT INTO forums (slug, name) VALUES ('a', 'A'), ('b', 'B');
         INSERT INTO users (email) VALUES ('u@example.com');
         INSERT INTO topics (forum_id, author_id, title, last_post_at) VALUES
           (1, 1, '出售 Netflix 会员', now()), (1, 1, '求购 ChatGPT Plus', now()),
           (2, 1, '港版 iPhone 未拆封', now()), (2, 1, 'ＵＳＤＴ 汇率讨论', now());",
    )
    .execute(db)
    .await
    .unwrap();
    let mut ids = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        let topic = (i % 4 + 1) as i64;
        let floor = (i / 4 + 1) as i32;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO posts (topic_id, floor, author_id, html, plain_text) VALUES ($1, $2, 1, '', $3) RETURNING id",
        )
        .bind(topic)
        .bind(floor)
        .bind(t)
        .fetch_one(db)
        .await
        .unwrap();
        ids.push(id);
    }
    ids
}

async fn post_ids(db: &PgPool, q: &str, before: Option<i64>) -> Vec<i64> {
    let (query, _) = groonga_query(q).unwrap();
    search::post_hits(db, &query, before, 20).await.unwrap().ids
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn chinese_and_partial_latin_terms_match(db: PgPool) {
    let ids = seed(
        &db,
        &[
            "出售奈飞会员，自动发货",
            "求购 ChatGPT Plus 账号",
            "港版未拆封，可走担保",
            "今天 ＵＳＤＴ 汇率多少",
            "担保交易已完成，好评",
        ],
    )
    .await;
    // 中文不需要词典：词中间、单字都能搜到
    assert_eq!(post_ids(&db, "自动发货", None).await, [ids[0]]);
    assert_eq!(post_ids(&db, "拆封", None).await, [ids[2]]);
    assert_eq!(post_ids(&db, "饭", None).await, Vec::<i64>::new());
    // 字母按 N 元组切分：词中间也能命中；大小写、全角半角不敏感
    assert_eq!(post_ids(&db, "gpt", None).await, [ids[1]]);
    assert_eq!(post_ids(&db, "CHATgpt", None).await, [ids[1]]);
    assert_eq!(post_ids(&db, "usdt", None).await, [ids[3]]);
    // 多个词是「与」；新的在前；游标之后只取更早的
    assert_eq!(post_ids(&db, "担保", None).await, [ids[4], ids[2]]);
    assert_eq!(post_ids(&db, "担保 好评", None).await, [ids[4]]);
    assert_eq!(post_ids(&db, "担保", Some(ids[4])).await, [ids[2]]);
    // 用户输入里的查询语法不生效：OR 与减号都只是普通文字
    assert_eq!(post_ids(&db, "担保 OR 汇率", None).await, Vec::<i64>::new());
    assert_eq!(post_ids(&db, "担保 -好评", None).await, Vec::<i64>::new());
    let (query, _) = groonga_query("担保").unwrap();
    assert_eq!(
        search::post_hits(&db, &query, None, 20)
            .await
            .unwrap()
            .total,
        2
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn rows_exclude_deleted_hidden_and_invisible_boards(db: PgPool) {
    let ids = seed(&db, &["关键词 一", "关键词 二", "关键词 三", "关键词 四"]).await;
    sqlx::raw_sql(
        "UPDATE posts SET status = 1 WHERE topic_id = 1; UPDATE topics SET flags = 8 WHERE id = 2;",
    )
    .execute(&db)
    .await
    .unwrap();
    let hits = post_ids(&db, "关键词", None).await;
    assert_eq!(hits, [ids[3], ids[2], ids[1], ids[0]]);
    // 板块 2（主题 3、4）可见：删除的楼层与隐藏主题的楼层不返回
    let rows = search::post_rows(&db, &hits, &[1, 2]).await.unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        [ids[3], ids[2]]
    );
    assert_eq!(rows[0].title, "ＵＳＤＴ 汇率讨论");
    // 只看得到板块 1 时一条都没有
    assert!(
        search::post_rows(&db, &hits, &[1])
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn topic_titles_filter_boards_in_groonga(db: PgPool) {
    seed(&db, &[]).await;
    let (q, _) = groonga_query("iphone").unwrap();
    assert_eq!(
        search::topic_hits(&db, &q, &[1, 2], None, 20)
            .await
            .unwrap()
            .ids,
        [3]
    );
    assert!(
        search::topic_hits(&db, &q, &[1], None, 20)
            .await
            .unwrap()
            .ids
            .is_empty()
    );
    assert!(
        search::topic_hits(&db, &q, &[], None, 20)
            .await
            .unwrap()
            .ids
            .is_empty()
    );
    let (q, _) = groonga_query("会员").unwrap();
    assert_eq!(
        search::topic_hits(&db, &q, &[1], None, 20)
            .await
            .unwrap()
            .ids,
        [1]
    );
    // 移动主题后索引里的板块随之更新
    sqlx::query("UPDATE topics SET forum_id = 2 WHERE id = 1")
        .execute(&db)
        .await
        .unwrap();
    let ids = search::topic_hits(&db, &q, &[2], None, 20)
        .await
        .unwrap()
        .ids;
    assert!(ids.contains(&1), "{ids:?}");
}
