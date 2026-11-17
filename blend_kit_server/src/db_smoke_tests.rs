//! 数据库查询的冒烟测试. SQL 全是运行期字符串, 写错的列名与类型编译器看不到, 这里把每条查询在
//! 真库上至少跑一遍 (写入再读回), 多数经路由走, 连鉴权中间件与 handler 一起过.
//! 只在独立 PostgreSQL 上执行, 每次用隔离的 schema; 只用固定数据, 不断言 master 的内容, 不联网
//! (会去取现货指数的接口 —— 基准列表、估价、询价存档、MCP 的工具调用 —— 不在这里调;
//! 估价快照与询价表另有各自的库测试).

use super::*;
use crate::test_db::TestDb;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const USER: &str = "test-session-token";
const ADMIN: &str = "admin-session-token-0123456789abcdef";

/// 请求带什么凭据.
#[derive(Clone, Copy)]
enum By<'a> {
    Nobody,
    /// 会话 Cookie.
    Session(&'a str),
    /// `X-API-Key` 请求头.
    ApiKey(&'a str),
    /// `Authorization: Bearer`.
    Bearer(&'a str),
}

fn app_on(pool: &PgPool) -> Router {
    app_with_state(
        PathBuf::from("missing-public"),
        AppState {
            auth: AuthConfig::new("tester", "secret", USER, false).with_admin(
                "boss",
                "admin-secret",
                ADMIN,
            ),
            database: Some(pool.clone()),
            rank_k: Arc::default(),
        },
    )
}

/// 发一个请求, 回 (状态码, JSON 体; 不是 JSON 记 Null).
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    by: By<'_>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    let builder = match by {
        By::Nobody => builder,
        By::Session(token) => builder.header(header::COOKIE, format!("doudou_session={token}")),
        By::ApiKey(key) => builder.header(DATA_API_KEY_HEADER, key),
        By::Bearer(key) => builder.header(header::AUTHORIZATION, format!("Bearer {key}")),
    };
    let body = body.map_or_else(Body::empty, |value| Body::from(value.to_string()));
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// 用户状态 (`web_user_state`): 没存过回默认值; 存了读回逐字段相同; 老前端不带一铲吨数时不冲掉已设的值.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_user_state_round_trips() {
    let db = TestDb::migrated("smoke_state").await;
    let app = app_on(&db.pool);
    let user = By::Session(USER);

    let (status, fresh) = call(&app, "GET", "/api/storage", user, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fresh["initialized"], false);

    let saved = json!({
        "coal_prefs": {"甲": {"enabled": true, "props_override": {"S": 0.61}}},
        "contract": [{"indicator": "S", "direction": "Upper", "max": 1.0}],
        "quantity": 1234.5,
        "user_coals": [{"name": "自建", "status": "active", "props": {"S": 0.7}}],
        "shovel_tons": 5.0
    });
    let (status, echoed) = call(&app, "PUT", "/api/storage", user, Some(saved.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(echoed["initialized"], true);
    let (_, loaded) = call(&app, "GET", "/api/storage", user, None).await;
    for key in [
        "coal_prefs",
        "contract",
        "quantity",
        "user_coals",
        "shovel_tons",
    ] {
        assert_eq!(loaded[key], saved[key], "{key}");
    }
    assert_eq!(loaded["initialized"], true);

    // 不带 shovel_tons 的保存 (上线前的老前端) 不把已设的值冲成空; 合同置空照常生效.
    let mut older = saved.clone();
    older.as_object_mut().unwrap().remove("shovel_tons");
    older["contract"] = Value::Null;
    older["quantity"] = json!(2000.0);
    let (status, _) = call(&app, "PUT", "/api/storage", user, Some(older)).await;
    assert_eq!(status, StatusCode::OK);
    let stored = database::load_storage(&db.pool, "tester").await.unwrap();
    assert_eq!(stored.shovel_tons, Some(5.0));
    assert_eq!(stored.quantity, 2000.0);
    assert!(stored.contract.is_none());

    // 管理员会话读写的是同一行.
    let (_, as_admin) = call(&app, "GET", "/api/storage", By::Session(ADMIN), None).await;
    assert_eq!(as_admin["quantity"], 2000.0);
    // 别的账号名下没有数据.
    let other = database::load_storage(&db.pool, "someone-else")
        .await
        .unwrap();
    assert!(!other.initialized);
    db.drop().await;
}

/// 历史方案 (`web_blend_history`): 保存、计数、列出、回填实测、导入旧记录、管理端读实测、清空.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_history_round_trips() {
    let db = TestDb::migrated("smoke_history").await;
    let app = app_on(&db.pool);
    let user = By::Session(USER);

    let blend = json!({
        "result": {"ok": true, "cost": {"cif_per_ton": 1500.5}, "recipe": {"甲": 0.6, "乙": 0.4}},
        "contract_name": " 测试合同 ",
        "quantity": 1000.0
    });
    let (status, created) = call(&app, "POST", "/api/history", user, Some(blend)).await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap().to_string();

    let (_, count) = call(&app, "GET", "/api/history/count", user, None).await;
    assert_eq!(count, json!({"count": 1}));
    let (_, list) = call(&app, "GET", "/api/history", user, None).await;
    assert_eq!(list[0]["id"], id.as_str());
    assert_eq!(list[0]["contract_name"], "测试合同");
    assert_eq!(list[0]["cost_cif"], 1500.5);
    assert_eq!(list[0]["recipe"], json!({"甲": 0.6, "乙": 0.4}));
    assert_eq!(list[0]["result"]["cost"]["cif_per_ton"], 1500.5);
    assert!(list[0]["csr_measured"].is_null());

    // 回填整张化验单: 13 列各写一次; 之后只补一项时别的不动.
    let assay = json!({
        "s": 0.8, "a": 9.5, "v": 25.0, "g": 80.0, "y": 16.0, "m": 8.0, "csr": 62.5,
        "cri": 25.0, "m40": 85.0, "m10": 6.0,
        "bulk_density": 1.0, "coking_hours": 20.0, "flue_temp": 1300.0
    });
    let measured = format!("/api/history/{id}/measured");
    let (status, updated) = call(&app, "PATCH", &measured, user, Some(assay.clone())).await;
    assert_eq!(
        (status, updated),
        (StatusCode::OK, json!({"updated": true}))
    );
    let (status, _) = call(&app, "PATCH", &measured, user, Some(json!({"csr": 63.0}))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, list) = call(&app, "GET", "/api/history", user, None).await;
    assert_eq!(list[0]["csr_measured"], 63.0);
    for (column, key) in [
        ("s_measured", "s"),
        ("a_measured", "a"),
        ("v_measured", "v"),
        ("g_measured", "g"),
        ("y_measured", "y"),
        ("m_measured", "m"),
        ("cri_measured", "cri"),
        ("m40_measured", "m40"),
        ("m10_measured", "m10"),
        ("bulk_density", "bulk_density"),
        ("coking_hours", "coking_hours"),
        ("flue_temp", "flue_temp"),
    ] {
        assert_eq!(list[0][column], assay[key], "{column}");
    }
    let missing = "/api/history/no-such-id/measured";
    let (status, _) = call(&app, "PATCH", missing, user, Some(json!({"csr": 60.0}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 导入旧记录: 同一个 id 第二次不重复写.
    let legacy = json!({"entries": [{
        "id": "legacy-1", "occurred_at": "2026-10-01T08:00:00Z", "cost_cif": 1400.0,
        "recipe": {"丙": 1.0}, "contract_name": "旧方案", "result": null,
        "csr_measured": 60.0, "s_measured": 0.9, "a_measured": 10.0, "v_measured": 26.0,
        "g_measured": 78.0, "y_measured": 15.0, "m_measured": 9.0
    }]});
    let import = "/api/history/import";
    let (status, imported) = call(&app, "POST", import, user, Some(legacy.clone())).await;
    assert_eq!((status, imported), (StatusCode::OK, json!({"imported": 1})));
    let (_, again) = call(&app, "POST", import, user, Some(legacy)).await;
    assert_eq!(again, json!({"imported": 0}));
    let (_, list) = call(&app, "GET", "/api/history", user, None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    // 按时间倒序: 刚保存的在前, 导入的旧记录在后.
    assert_eq!(list[1]["id"], "legacy-1");
    assert_eq!(list[1]["g_measured"], 78.0);

    // 管理端的 k 标定与 CSR 模型读的是这些实测 (有实测 CSR 且存了结果的那些).
    let admin = By::Session(ADMIN);
    for uri in ["/api/admin/calibration", "/api/admin/csr-model"] {
        let (status, body) = call(&app, "GET", uri, admin, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(body["ok"], true, "{uri}");
    }

    let (status, cleared) = call(&app, "DELETE", "/api/history", user, None).await;
    assert_eq!((status, cleared), (StatusCode::OK, json!({"deleted": 2})));
    assert_eq!(
        database::count_history(&db.pool, "tester").await.unwrap(),
        0
    );
    db.drop().await;
}

/// 密钥 (`data_api_keys`, 含 `scope`): 生成、列出、鉴权、销毁; 一把钥匙只开一扇门 ——
/// data 用途的进不了 `/mcp`, mcp 用途的碰不了煤库.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_key_scopes_open_only_their_own_door() {
    let db = TestDb::migrated("smoke_keys").await;
    let app = app_on(&db.pool);
    let admin = By::Session(ADMIN);
    let keys = "/api/admin/keys";

    let (status, data) = call(&app, "POST", keys, admin, Some(json!({"name": "录数据"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(data["key"]["scope"], "data");
    let phone = json!({"name": "手机", "scope": "mcp"});
    let (status, mcp) = call(&app, "POST", keys, admin, Some(phone)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(mcp["key"]["scope"], "mcp");
    let bad = json!({"name": "别的", "scope": "admin"});
    let (status, _) = call(&app, "POST", keys, admin, Some(bad)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let data_key = data["plaintext"].as_str().unwrap().to_string();
    let mcp_key = mcp["plaintext"].as_str().unwrap().to_string();
    let mcp_id = mcp["key"]["id"].as_str().unwrap().to_string();

    // 列表里有两把, 没有明文也没有哈希.
    let (status, listed) = call(&app, "GET", keys, admin, None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = listed["keys"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let text = listed.to_string();
    assert!(
        !text.contains(&data_key) && !text.contains(&api_key::hash_key(&data_key)),
        "{text}"
    );

    let overrides = "/api/master/overrides";
    let tools = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let mcp_path = |key: &str| format!("/mcp/{key}");

    // data 用途: 能读写煤库覆盖层, 进不了 /mcp (请求头、路径两种带法都不行).
    let (status, body) = call(&app, "GET", overrides, By::ApiKey(&data_key), None).await;
    assert_eq!((status, &body["ok"]), (StatusCode::OK, &json!(true)));
    for (uri, by) in [
        ("/mcp".to_string(), By::Bearer(&data_key)),
        ("/mcp".to_string(), By::ApiKey(&data_key)),
        (mcp_path(&data_key), By::Nobody),
    ] {
        let (status, body) = call(&app, "POST", &uri, by, Some(tools.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(body["ok"], false);
    }

    // mcp 用途: 进得了 /mcp (三种带法), 碰不了煤库的任何一个接口.
    for (uri, by) in [
        ("/mcp".to_string(), By::Bearer(&mcp_key)),
        ("/mcp".to_string(), By::ApiKey(&mcp_key)),
        (mcp_path(&mcp_key), By::Nobody),
    ] {
        let (status, body) = call(&app, "POST", &uri, by, Some(tools.clone())).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(body["result"]["tools"].as_array().unwrap().len(), 6);
    }
    for (method, uri) in [
        ("POST", "/api/master/coals"),
        ("GET", overrides),
        ("DELETE", "/api/master/coals/SMOKE/Y"),
    ] {
        let update = json!({"updates": []});
        let (status, body) = call(&app, method, uri, By::ApiKey(&mcp_key), Some(update)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        assert_eq!(body["ok"], false);
    }
    // 密钥也当不了管理员、当不了登录会话.
    for uri in [keys, "/api/storage"] {
        let (status, _) = call(&app, "GET", uri, By::ApiKey(&data_key), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }

    // 库这一层同样按用途对: 哈希对、用途不对查不到.
    let hash = api_key::hash_key(&mcp_key);
    for (scope, found) in [(api_key::Scope::Mcp, true), (api_key::Scope::Data, false)] {
        let holder = database::authenticate_api_key(&db.pool, &hash, scope)
            .await
            .unwrap();
        assert_eq!(holder.as_deref(), found.then_some("手机"), "{scope:?}");
    }
    // 假密钥: 配了库时回 401 (路由表测试里没配库, 那里是 503).
    let (status, _) = call(
        &app,
        "POST",
        "/mcp/not-a-real-key",
        By::Nobody,
        Some(tools.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 用过的密钥记了最近使用时间; 销毁后立即失效, 再销毁回 404.
    let (_, listed) = call(&app, "GET", keys, admin, None).await;
    for row in listed["keys"].as_array().unwrap() {
        assert!(row["last_used_at"].is_string(), "{row}");
        assert!(row["revoked_at"].is_null(), "{row}");
    }
    let revoke = format!("{keys}/{mcp_id}");
    let (status, _) = call(&app, "DELETE", &revoke, admin, None).await;
    assert_eq!(status, StatusCode::OK);
    let by = By::Bearer(&mcp_key);
    let (status, _) = call(&app, "POST", "/mcp", by, Some(tools)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for uri in [revoke.as_str(), "/api/admin/keys/not-a-uuid"] {
        let (status, _) = call(&app, "DELETE", uri, admin, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    let (_, listed) = call(&app, "GET", keys, admin, None).await;
    let revoked: Vec<&Value> = listed["keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["revoked_at"].is_string())
        .collect();
    assert_eq!(revoked.len(), 1);
    assert_eq!(revoked[0]["id"], mcp_id.as_str());
    db.drop().await;
}

/// 煤库覆盖层 (`coal_overrides`): 写入、读回、改写、回滚. 煤名是编的 —— 这里不经
/// `POST /api/master/coals` 的校验 (它要对照 master 的内容), 直接走落库的那一层.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_coal_overrides_round_trip() {
    let db = TestDb::migrated("smoke_overrides").await;
    let app = app_on(&db.pool);
    let admin = By::Session(ADMIN);
    let update = |value: f64| master_data::CoalUpdate {
        coal: "SMOKE".into(),
        field: "Y".into(),
        value,
        source: "冒烟测试".into(),
        confidence: "medium".into(),
    };

    let written = database::upsert_coal_overrides(&db.pool, &[update(16.0)], "管理员:boss")
        .await
        .unwrap();
    assert_eq!(written, 1);
    database::upsert_coal_overrides(&db.pool, &[update(17.5)], "录数据")
        .await
        .unwrap();
    let (status, listed) = call(&app, "GET", "/api/master/overrides", admin, None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = listed["overrides"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "同一格改写不新增行");
    assert_eq!(rows[0]["coal_name"], "SMOKE");
    assert_eq!(rows[0]["field"], "Y");
    assert_eq!(rows[0]["value"], 17.5);
    assert_eq!(rows[0]["source"], "冒烟测试");
    assert_eq!(rows[0]["confidence"], "medium");

    // 有覆盖层时 /api/master 照样给得出一份煤库 (覆盖的是不存在的煤, 合并不了就退回基线).
    let (status, master) = call(&app, "GET", "/api/master", By::Session(USER), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(master.is_object());

    let rollback = "/api/master/coals/SMOKE/Y";
    let (status, removed) = call(&app, "DELETE", rollback, admin, None).await;
    assert_eq!(
        (status, removed),
        (StatusCode::OK, json!({"ok": true, "removed": 1}))
    );
    let (status, _) = call(&app, "DELETE", rollback, admin, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(database::list_coal_overrides(&db.pool)
        .await
        .unwrap()
        .is_empty());
    db.drop().await;
}

/// 全局设置 (`app_settings` 里的煤阶交互 k): 写入、读回、启动时读、关闭即删行.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_settings_round_trip() {
    let db = TestDb::migrated("smoke_settings").await;
    let app = app_on(&db.pool);
    let admin = By::Session(ADMIN);
    let settings = "/api/admin/settings";
    let k_of = |body: &Value| body["settings"]["rank_interaction_k"].clone();

    let (status, body) = call(&app, "GET", settings, admin, None).await;
    assert_eq!((status, k_of(&body)), (StatusCode::OK, Value::Null));
    assert_eq!(admin::load_rank_interaction_k(&db.pool).await, None);

    for k in [50.0, 66.5] {
        let put = json!({"rank_interaction_k": k});
        let (status, body) = call(&app, "PUT", settings, admin, Some(put)).await;
        assert_eq!((status, k_of(&body)), (StatusCode::OK, json!(k)));
        let (_, body) = call(&app, "GET", settings, admin, None).await;
        assert_eq!(k_of(&body), json!(k));
        assert_eq!(admin::load_rank_interaction_k(&db.pool).await, Some(k));
    }

    let off = json!({"rank_interaction_k": null});
    let (status, body) = call(&app, "PUT", settings, admin, Some(off)).await;
    assert_eq!((status, k_of(&body)), (StatusCode::OK, Value::Null));
    assert_eq!(admin::load_rank_interaction_k(&db.pool).await, None);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "关闭即删行, 库里不留 0 或 null");
    db.drop().await;
}

/// 基准报价 (`price_benchmarks`): 管理员补录、读回 (含产地三栏与录入人)、删除.
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_benchmarks_round_trip() {
    let db = TestDb::migrated("smoke_benchmarks").await;
    let app = app_on(&db.pool);
    let admin = By::Session(ADMIN);
    let create = "/api/admin/benchmarks";

    let full = json!({
        "name": "冒烟基准", "coal_type": "焦煤", "form": "精煤",
        "province": "山西省", "city": "晋中市", "county": "灵石县",
        "props": {"S": 1.3}, "price": 2050.5, "date": "2026-10-16", "source": "测试录入"
    });
    let (status, created) = call(&app, "POST", create, admin, Some(full)).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    // 只有必填项的也能存: 其余各栏为空.
    let bare =
        json!({"name": "只有必填", "props": {"S": 0.6}, "price": 1900, "date": "2026-10-17"});
    let (status, _) = call(&app, "POST", create, admin, Some(bare)).await;
    assert_eq!(status, StatusCode::OK);

    let stored = benchmarks::stored_for_test(&db.pool).await;
    assert_eq!(stored.len(), 2);
    let row = stored.iter().find(|row| row["id"] == id.as_str()).unwrap();
    for (key, expected) in [
        ("name", json!("冒烟基准")),
        ("coal_type", json!("焦煤")),
        ("form", json!("精煤")),
        ("province", json!("山西省")),
        ("city", json!("晋中市")),
        ("county", json!("灵石县")),
        ("props", json!({"S": 1.3})),
        ("price", json!(2050.5)),
        ("date", json!("2026-10-16")),
        ("source", json!("测试录入")),
        ("created_by", json!("boss")),
        ("origin", json!("stored")),
    ] {
        assert_eq!(row[key], expected, "{key}");
    }
    let bare = stored.iter().find(|row| row["name"] == "只有必填").unwrap();
    assert!(bare.get("coal_type").is_none() && bare.get("province").is_none());

    let delete = format!("/api/admin/benchmarks/{id}");
    let (status, _) = call(&app, "DELETE", &delete, admin, None).await;
    assert_eq!(status, StatusCode::OK);
    for uri in [delete.as_str(), "/api/admin/benchmarks/not-a-uuid"] {
        let (status, _) = call(&app, "DELETE", uri, admin, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    assert_eq!(benchmarks::stored_for_test(&db.pool).await.len(), 1);
    db.drop().await;
}

/// 只读库的那几个接口在空库上都答得出来 (列名、类型写错的话这里就 500 了).
#[tokio::test]
#[ignore = "需要独立 PostgreSQL, 设置 DATABASE_URL 后用 --ignored 执行"]
async fn test_read_endpoints_answer_on_an_empty_database() {
    let db = TestDb::migrated("smoke_reads").await;
    let app = app_on(&db.pool);

    let (status, _) = call(&app, "GET", "/api/health", By::Nobody, None).await;
    assert_eq!(status, StatusCode::OK);
    for (uri, by) in [
        ("/api/coal-prices", By::Session(USER)),
        ("/api/coal-prices/history?name=SMOKE", By::Session(USER)),
        ("/api/quotes/latest", By::Session(USER)),
        ("/api/mcci", By::Session(USER)),
        ("/api/admin/quotes", By::Session(ADMIN)),
        ("/api/admin/quotes?limit=5", By::Session(ADMIN)),
        ("/api/admin/calibration", By::Session(ADMIN)),
        ("/api/admin/csr-model", By::Session(ADMIN)),
        ("/api/master/overrides", By::Session(ADMIN)),
    ] {
        let (status, body) = call(&app, "GET", uri, by, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["ok"], true, "{uri}: {body}");
    }
    let missing = "/api/admin/quotes/00000000-0000-4000-8000-000000000000";
    let (status, _) = call(&app, "DELETE", missing, By::Session(ADMIN), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    db.drop().await;
}
