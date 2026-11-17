//! 库测试的支架: 在独立 PostgreSQL 上每次开一个隔离的 schema, 按生产启动的办法
//! ([`crate::database::migrator`]) 把 `migrations/` 里的全部迁移依次跑一遍.
//! 只给 `#[ignore]` 的库测试用 (设置 `DATABASE_URL` 后 `--ignored` 执行).

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) struct TestDb {
    admin: PgPool,
    pub(crate) pool: PgPool,
    schema: String,
}

impl TestDb {
    /// 新开一个 schema (名字带 `prefix`), 连接都落在它里面, 并跑完全部迁移.
    pub(crate) async fn migrated(prefix: &str) -> Self {
        let url = std::env::var("DATABASE_URL").expect("请设置独立测试库 DATABASE_URL");
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("{prefix}_{}", Uuid::new_v4().simple());
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
        crate::database::migrator().run(&pool).await.unwrap();
        Self {
            admin,
            pool,
            schema,
        }
    }

    /// 用完删掉这个 schema.
    pub(crate) async fn drop(self) {
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
