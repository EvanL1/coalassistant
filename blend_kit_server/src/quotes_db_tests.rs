//! 只在本机独立 PostgreSQL 上执行, 每次使用隔离 schema, 不读取 master 数据.

use super::*;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

const QUOTES: &str = include_str!("../migrations/0013_coal_quotes.sql");
/// 0015 同时改基准报价表, 得先有它.
const PLACE_PREREQUISITES: &str = concat!(
    include_str!("../migrations/0007_price_benchmarks.sql"),
    include_str!("../migrations/0008_price_benchmark_form_region.sql"),
);
const PLACE: &str = include_str!("../migrations/0015_place_fields.sql");

struct Isolated {
    admin: PgPool,
    pool: PgPool,
    schema: String,
}

/// 独立 schema 里按顺序执行给定的迁移; 用完由调用方 `drop_schema`.
async fn isolated(migrations: &[&str]) -> Isolated {
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
    for migration in migrations {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    Isolated {
        admin,
        pool,
        schema,
    }
}

async fn drop_schema(isolated: Isolated) {
    isolated.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {} CASCADE", isolated.schema))
        .execute(&isolated.admin)
        .await
        .unwrap();
}

/// 0015: 旧写法的产地回填成省 / 市 / 县, 认不出的原样留着; 之后新存的询价三个字段读写一致.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_place_migration_backfills_and_new_quotes_round_trip() {
    let isolated = isolated(&[QUOTES, PLACE_PREREQUISITES]).await;
    let pool = &isolated.pool;
    sqlx::raw_sql(
        "INSERT INTO coal_quotes (id, username, batch_id, quoted_on, name, region, fob, props) VALUES
           ('q1', 'u', 'old', '2026-11-02', '只写省', '山西', 2000, '{}'),
           ('q2', 'u', 'old', '2026-11-02', '写到县', '山西灵石', 2000, '{}'),
           ('q3', 'u', 'old', '2026-11-02', '认不出', '某个地方', 2000, '{}'),
           ('q4', 'u', 'old', '2026-11-02', '没写', NULL, 2000, '{}');
         INSERT INTO price_benchmarks (id, name, region, props, price, quoted_on, created_by) VALUES
           ('b1', '旧基准', '山西临汾', '{\"S\": 1.0}', 2000, '2026-10-01', 'admin');",
    )
    .execute(pool)
    .await
    .unwrap();
    // 跑两遍: 重复执行无害.
    for _ in 0..2 {
        sqlx::raw_sql(PLACE).execute(pool).await.unwrap();
    }

    let day: NaiveDate = "2026-11-12".parse().unwrap();
    let batch: QuoteBatch = serde_json::from_value(json!({
        "batch_id": "new", "items": [
            {"name": "新煤", "fob": 2100, "props": {"S": 0.6},
             "province": "山西省", "city": "晋中市", "county": "灵石县"},
            {"name": "没填产地", "fob": 2100, "props": {"S": 0.6}}
        ]
    }))
    .unwrap();
    replace_batch(
        pool,
        "u",
        day,
        &batch,
        &[Estimate::default(), Estimate::default()],
    )
    .await
    .unwrap();

    let records: Vec<QuoteRecord> =
        sqlx::query_as("SELECT * FROM coal_quotes ORDER BY batch_id DESC, id")
            .fetch_all(pool)
            .await
            .unwrap();
    let benchmarks = benchmarks::stored_for_test(pool).await;
    drop_schema(isolated).await;

    let place = |name: &str| {
        let record = records.iter().find(|record| record.name == name).unwrap();
        (
            record.region.clone(),
            record.province.clone(),
            record.city.clone(),
            record.county.clone(),
        )
    };
    let text = |value: &str| Some(value.to_string());
    assert_eq!(place("只写省"), (text("山西"), text("山西省"), None, None));
    assert_eq!(
        place("写到县"),
        (
            text("山西灵石"),
            text("山西省"),
            text("晋中市"),
            text("灵石县")
        )
    );
    assert_eq!(place("认不出"), (text("某个地方"), None, None, None));
    assert_eq!(place("没写"), (None, None, None, None));
    assert_eq!(
        place("新煤"),
        (None, text("山西省"), text("晋中市"), text("灵石县"))
    );
    assert_eq!(place("没填产地"), (None, None, None, None));
    // 基准报价表同样回填, 读出来的基准带省市、能直接给核心用.
    assert_eq!(benchmarks.len(), 1);
    assert_eq!(benchmarks[0]["province"], "山西省");
    assert_eq!(benchmarks[0]["city"], "临汾市");
    assert!(benchmarks[0].get("county").is_none());
    let parsed: blend_kit::Benchmark = serde_json::from_value(benchmarks[0].clone()).unwrap();
    assert_eq!(parsed.city.as_deref(), Some("临汾市"));
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_replace_quote_batch_serializes_concurrent_writers() {
    let Isolated {
        admin,
        pool,
        schema,
    } = isolated(&[QUOTES, PLACE_PREREQUISITES, PLACE]).await;

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
    drop_schema(Isolated {
        admin: admin.clone(),
        pool,
        schema: schema.clone(),
    })
    .await;

    assert!(blocked.is_ok(), "后到请求未进入锁等待");
    written.unwrap();
    assert_eq!(names, vec!["后到煤"], "同批覆盖不能留下两个请求的并集");
}
