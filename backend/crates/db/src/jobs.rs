//! worker 后台任务进度（按主键顺序扫描的任务处理到哪里）。

use sqlx::PgConnection;

/// 取任务进度并加锁到事务结束；另一个 worker 正在处理时返回 `None`。
pub async fn lock_cursor(conn: &mut PgConnection, name: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query!(
        "INSERT INTO job_cursors (name) VALUES ($1) ON CONFLICT (name) DO NOTHING",
        name
    )
    .execute(&mut *conn)
    .await?;
    let row = sqlx::query_file!("queries/job_cursor_lock.sql", name)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(|r| r.position))
}

pub async fn set_cursor(
    conn: &mut PgConnection,
    name: &str,
    position: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE job_cursors SET position = $2, updated_at = now() WHERE name = $1",
        name,
        position
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
