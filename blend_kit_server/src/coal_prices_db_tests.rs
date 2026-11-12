//! 只在本机独立 PostgreSQL 上执行, 每次使用隔离 schema, 不读取 master 数据.

use super::*;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn row(name: &str, price: f64) -> PriceRow {
    PriceRow {
        name: name.into(),
        price,
        points: 1000.0,
        benchmark: Some("基准".into()),
        benchmark_price: Some(price),
        missing: vec![],
    }
}

async fn sources(pool: &PgPool, day: NaiveDate) -> Vec<(String, f64, String)> {
    sqlx::query_as("SELECT name, price, source FROM coal_price_daily WHERE day = $1 ORDER BY name")
        .bind(day)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_partial_snapshots_never_replace_complete_ones_and_get_replaced_later() {
    let url = std::env::var("DATABASE_URL").expect("请设置独立测试库 DATABASE_URL");
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("price_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let session_schema = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            let schema = session_schema.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    for migration in [
        include_str!("../migrations/0011_coal_price_daily.sql"),
        include_str!("../migrations/0012_coal_price_source.sql"),
        include_str!("../migrations/0014_coal_price_partial.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }

    let today = beijing_day(Utc::now());
    let yesterday = today.pred_opt().unwrap();
    let as_tuple =
        |name: &str, price: f64, source: &str| (name.to_string(), price, source.to_string());

    // 当天还没有快照: 临时快照照写, 之后的临时快照也能覆盖它.
    assert!(write_snapshot(&pool, today, &[row("甲", 1.0)], PARTIAL)
        .await
        .unwrap());
    assert!(write_snapshot(&pool, today, &[row("甲", 2.0)], PARTIAL)
        .await
        .unwrap());
    assert_eq!(
        sources(&pool, today).await,
        [as_tuple("甲", 2.0, "partial")]
    );
    // 完整快照覆盖临时的; 之后输入又不全时不覆盖完整的.
    assert!(write_snapshot(&pool, today, &[row("甲", 3.0)], LIVE)
        .await
        .unwrap());
    assert!(!write_snapshot(&pool, today, &[row("甲", 4.0)], PARTIAL)
        .await
        .unwrap());
    assert_eq!(sources(&pool, today).await, [as_tuple("甲", 3.0, "live")]);

    // 过了零点的临时快照: 补算视为缺, 并替换它; 完整的实时行不动.
    sqlx::query(
        "INSERT INTO coal_price_daily (day, name, price, points, missing, source)
         VALUES ($1, '甲', 5, 1000, '[]', 'partial'), ($1, '乙', 6, 1000, '[]', 'live')",
    )
    .bind(yesterday)
    .execute(&pool)
    .await
    .unwrap();
    let existing = existing_keys(&pool, (yesterday, yesterday)).await.unwrap();
    assert_eq!(existing, HashSet::from([(yesterday, "乙".to_string())]));
    let inserted = insert_backfill(
        &pool,
        &[(yesterday, row("甲", 7.0)), (yesterday, row("乙", 8.0))],
    )
    .await
    .unwrap();
    // 数据库的排序规则因环境而异, 在这里按字节序重排后再比.
    let mut after = sources(&pool, yesterday).await;
    after.sort_by(|a, b| a.0.cmp(&b.0));
    let history = read_history(&pool, "甲").await.unwrap();

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();

    assert_eq!(inserted, 1);
    assert_eq!(
        after,
        [as_tuple("乙", 6.0, "live"), as_tuple("甲", 7.0, "backfill")]
    );
    assert!(history
        .iter()
        .all(|point| point.source == "live" || point.source == "backfill"));
}
