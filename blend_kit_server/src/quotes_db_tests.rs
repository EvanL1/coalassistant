//! 只在本机独立 PostgreSQL 上执行, 每次使用隔离 schema, 不读取 master 数据.

use super::*;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_replace_quote_batch_serializes_concurrent_writers() {
    let url = std::env::var("DATABASE_URL").expect("请设置独立测试库 DATABASE_URL");
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("quote_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let session_schema = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |connection, _| {
            let schema = session_schema.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('search_path', $1, false), set_config('application_name', $1, false)")
                    .bind(schema)
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0013_coal_quotes.sql"))
        .execute(&pool)
        .await
        .unwrap();

    let day: NaiveDate = "2026-10-31".parse().unwrap();
    let batch: QuoteBatch = serde_json::from_value(json!({
        "batch_id": "concurrent", "items": [{"name": "旧煤", "fob": 2000, "props": {"S": 0.6}}]
    }))
    .unwrap();
    replace_batch(&pool, "tester", day, &batch, &[Estimate::default()])
        .await
        .unwrap();

    // 先到事务暂不提交, 让后到请求必然在锁上等待, 不靠随机调度制造重叠.
    let mut first = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))")
        .bind("tester")
        .bind("concurrent")
        .execute(&mut *first)
        .await
        .unwrap();
    sqlx::query("DELETE FROM coal_quotes WHERE username = 'tester' AND batch_id = 'concurrent'")
        .execute(&mut *first)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO coal_quotes (id, username, batch_id, quoted_on, name, fob, props)
         VALUES ('first', 'tester', 'concurrent', '2026-10-31', '先到煤', 2100, '{}')",
    )
    .execute(&mut *first)
    .await
    .unwrap();

    let writer_pool = pool.clone();
    let second = tokio::spawn(async move {
        let mut replacement = batch;
        replacement.items[0].name = "后到煤".into();
        replace_batch(
            &writer_pool,
            "tester",
            day,
            &replacement,
            &[Estimate::default()],
        )
        .await
    });
    let blocked = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                 WHERE application_name = $1 AND wait_event_type = 'Lock')",
            )
            .bind(&schema)
            .fetch_one(&admin)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    first.commit().await.unwrap();
    let written = second.await.unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM coal_quotes ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();

    assert!(blocked.is_ok(), "后到请求未进入锁等待");
    written.unwrap();
    assert_eq!(names, vec!["后到煤"], "同批覆盖不能留下两个请求的并集");
}
