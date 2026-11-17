//! 服务端防护: 登录限速、安全响应头、MCP 批量上限、令牌长度提醒.

use super::*;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn auth(secure_cookie: bool) -> AuthConfig {
    AuthConfig::new(
        "tester",
        "secret",
        "test-session-token-0123456789abcdef",
        secure_cookie,
    )
}

fn app(secure_cookie: bool) -> Router {
    app_with_auth(PathBuf::from("missing-public"), auth(secure_cookie))
}

fn app_with_ip_lockout() -> Router {
    app_with_auth(
        PathBuf::from("missing-public"),
        auth(false).with_login_ip_lockout(true),
    )
}

async fn fail_logins(router: &Router, ip: &str, times: usize) {
    let attempts: Vec<_> = (0..times)
        .map(|_| {
            let (router, ip) = (router.clone(), ip.to_string());
            tokio::spawn(async move { router.oneshot(login_from(&ip, "wrong")).await })
        })
        .collect();
    for attempt in attempts {
        assert_eq!(
            attempt.await.unwrap().unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

fn login_from(ip: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", format!("{ip}, 100.64.0.1"))
        .body(Body::from(
            json!({ "username": "tester", "password": password }).to_string(),
        ))
        .unwrap()
}

async fn body_json(response: Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// 默认不按来源 IP 锁: 代理头可能由客户端伪造, 锁定会被用来把别人关在门外.
#[tokio::test]
async fn test_ip_lockout_is_off_by_default() {
    let router = app(false);
    fail_logins(&router, "198.51.100.6", 6).await;
    let ok = router
        .oneshot(login_from("198.51.100.6", "secret"))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

/// 打开锁定后, 同一来源连错 5 次即锁: 回 429 + Retry-After, 连正确密码也不验; 别的来源照常登录.
#[tokio::test]
async fn test_repeated_login_failures_lock_out_only_that_source() {
    let router = app_with_ip_lockout();
    fail_logins(&router, "198.51.100.7", 5).await;

    let started = std::time::Instant::now();
    let locked = router
        .clone()
        .oneshot(login_from("198.51.100.7", "secret"))
        .await
        .unwrap();
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(started.elapsed() < LOGIN_FAILURE_DELAY);
    let retry: u64 = locked.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    let body = body_json(locked).await;
    assert_eq!(body["authenticated"], false);
    assert!(body["reason"].as_str().unwrap().contains("秒后再试"));
    assert!(body.get("admin").is_none());

    let other = router
        .oneshot(login_from("203.0.113.9", "secret"))
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::OK);
}

/// IPv6 按 /64 归桶: 同一 /64 里换地址也躲不开锁, 相邻的 /64 不受牵连.
#[tokio::test]
async fn test_ipv6_lockout_covers_the_whole_slash_64() {
    let router = app_with_ip_lockout();
    fail_logins(&router, "2001:db8:1:2::a", 5).await;
    let same_prefix = router
        .clone()
        .oneshot(login_from("2001:db8:1:2:ffff:1:2:3", "secret"))
        .await
        .unwrap();
    assert_eq!(same_prefix.status(), StatusCode::TOO_MANY_REQUESTS);
    let next_prefix = router
        .oneshot(login_from("2001:db8:1:3::a", "secret"))
        .await
        .unwrap();
    assert_eq!(next_prefix.status(), StatusCode::OK);
}

/// 成功登录清零: 错 4 次、对 1 次、再错 4 次, 都不锁.
#[tokio::test]
async fn test_successful_login_resets_failures() {
    let router = app_with_ip_lockout();
    for round in 0..2 {
        fail_logins(&router, "198.51.100.8", 4).await;
        let ok = router
            .clone()
            .oneshot(login_from("198.51.100.8", "secret"))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK, "第 {round} 轮");
    }
}

#[tokio::test]
async fn test_security_headers_on_api_and_static_responses() {
    for uri in ["/api/health", "/api/master", "/history"] {
        let response = app(false)
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let headers = response.headers();
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{uri}");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "SAMEORIGIN", "{uri}");
        assert_eq!(
            headers[header::CONTENT_SECURITY_POLICY],
            "frame-ancestors 'self'",
            "{uri}"
        );
        assert_eq!(headers[header::REFERRER_POLICY], "same-origin", "{uri}");
        assert!(
            headers.get(header::STRICT_TRANSPORT_SECURITY).is_none(),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn test_hsts_only_when_cookies_are_secure() {
    let response = app(true)
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::STRICT_TRANSPORT_SECURITY],
        "max-age=31536000"
    );
}

#[test]
fn test_mcp_batch_size_is_capped() {
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let (messages, batch) = mcp_messages(json!(vec![call.clone(); MAX_MCP_MESSAGES])).unwrap();
    assert_eq!((messages.len(), batch), (MAX_MCP_MESSAGES, true));
    let (messages, batch) = mcp_messages(call.clone()).unwrap();
    assert_eq!((messages.len(), batch), (1, false));

    let error = mcp_messages(json!(vec![call; MAX_MCP_MESSAGES + 1])).unwrap_err();
    assert_eq!(error["error"]["code"], -32600);
    assert!(error["id"].is_null());
}

#[test]
fn test_mcp_internal_errors_answer_every_request_with_an_id() {
    let messages = [
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call"}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": "abc", "method": "tools/call"}),
    ];
    let ids = messages
        .iter()
        .filter_map(|message| message.get("id").cloned())
        .collect();
    let replies = mcp_internal_errors(ids, "求解线程异常退出");
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["id"], 7);
    assert_eq!(replies[1]["id"], "abc");
    assert!(replies
        .iter()
        .all(|reply| reply["error"]["code"] == -32603 && reply["jsonrpc"] == "2.0"));
}

#[test]
fn test_short_session_token_is_flagged_but_accepted() {
    assert!(AuthConfig::new("u", "p", "short", false).short_session_token());
    assert!(!AuthConfig::new("u", "p", "x".repeat(32), false).short_session_token());
}

/// 非法指标在存档前就回 400, 说明哪种煤哪一项; 不会落进校准用的询价表.
#[tokio::test]
async fn test_quote_archive_rejects_bad_props_before_storing() {
    for (props, indicator) in [
        (json!({ "A": -5 }), "A"),
        (json!({ "A": "abc" }), "A"),
        (json!({ "A": { "v": 1 } }), "A"),
        (json!({ "S": 0.6, "Ro": 1.1 }), "Ro"),
    ] {
        let body = json!({ "batch_id": "abc-1", "items": [
            { "name": "好煤", "fob": 2000, "props": { "S": 0.6 } },
            { "name": "测试煤", "fob": 2150, "props": props },
        ] });
        let response = app(false)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/quotes")
                    .header(
                        header::COOKIE,
                        "doudou_session=test-session-token-0123456789abcdef",
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{props}");
        let reason = body_json(response).await["reason"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            reason.contains("测试煤") && reason.contains(indicator),
            "{props}: {reason}"
        );
    }
}

fn rpc(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// 只有求解/估价的消息进求解闸门: 闸门忙 (这里直接回超时) 时, 握手、列工具、查合同照常回.
#[tokio::test]
async fn test_mcp_only_computing_messages_wait_for_the_solver() {
    let messages = vec![
        rpc(1, "initialize", json!({})),
        rpc(2, "tools/list", json!({})),
        rpc(
            3,
            "tools/call",
            json!({ "name": "today_blend", "arguments": {} }),
        ),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        rpc(
            4,
            "tools/call",
            json!({ "name": "get_contract", "arguments": {} }),
        ),
        rpc(
            5,
            "tools/call",
            json!({ "name": "find_coal", "arguments": { "name": "x" } }),
        ),
    ];
    let runs = std::sync::atomic::AtomicUsize::new(0);
    let replies = mcp_replies(messages, None, |_task| {
        runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Err(limits::ComputeError::Timeout) }
    })
    .await;
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    let ids: Vec<_> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(ids, [json!(1), json!(2), json!(3), json!(4), json!(5)]);
    assert!(replies[0]["result"]["serverInfo"].is_object());
    assert!(replies[1]["result"]["tools"].is_array());
    for solving in [&replies[2], &replies[4]] {
        assert_eq!(solving["error"]["code"], -32603);
        assert!(solving["error"]["message"]
            .as_str()
            .unwrap()
            .contains("超时"));
    }
    // 查合同不进闸门: 这里没加载线上数据, 回的是"未加载数据"而不是超时.
    assert!(!replies[3]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("超时"));
}

#[tokio::test]
async fn test_mcp_without_computing_messages_skips_the_solver() {
    let messages = vec![rpc(1, "initialize", json!({})), rpc(2, "ping", json!({}))];
    let runs = std::sync::atomic::AtomicUsize::new(0);
    let replies = mcp_replies(messages, None, |_task| {
        runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Err(limits::ComputeError::Timeout) }
    })
    .await;
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(replies.len(), 2);
    assert!(replies.iter().all(|reply| reply.get("result").is_some()));
}

/// 闸门放行时, 求解消息在闸门里算, 结果按原顺序回.
#[tokio::test]
async fn test_mcp_computing_messages_run_inside_the_solver() {
    let messages = vec![
        rpc(
            1,
            "tools/call",
            json!({ "name": "today_blend", "arguments": {} }),
        ),
        rpc(2, "ping", json!({})),
    ];
    let replies = mcp_replies(messages, None, |task| async move { Ok(task()) }).await;
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["error"]["message"], "服务端未加载数据");
    assert_eq!(replies[1]["id"], 2);
    assert!(replies[1]["result"].is_object());
}

fn market_call(id: i64) -> Value {
    rpc(
        id,
        "tools/call",
        json!({ "name": "market", "arguments": {} }),
    )
}

/// 看行情只拼文字, 不占求解闸门: 闸门忙时照常回; 询价精算要求解, 进闸门.
#[tokio::test]
async fn test_mcp_market_skips_the_solver_but_quote_blend_waits() {
    let context = mcp::Context {
        master: json!({ "coals": [] }),
        storage: database::UserStorage {
            initialized: true,
            coal_prefs: json!({}),
            contract: None,
            quantity: 1000.0,
            user_coals: json!([]),
            shovel_tons: None,
        },
        benchmarks: Vec::new(),
        rank_k: None,
        coal_prices: None,
        market: Some(vec![json!({
            "code": "X", "type_name": "肥煤", "name": "肥煤指数",
            "points": [{ "date": "2026-11-13", "price": 1700.0 }]
        })]),
        today: "2026-11-14".parse().unwrap(),
    };
    let messages = vec![
        market_call(1),
        rpc(
            2,
            "tools/call",
            json!({ "name": "quote_blend", "arguments": {} }),
        ),
    ];
    let runs = std::sync::atomic::AtomicUsize::new(0);
    let replies = mcp_replies(messages, Some(context), |_task| {
        runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Err(limits::ComputeError::Timeout) }
    })
    .await;
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["isError"], false);
    assert!(replies[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("- 肥煤: 1700 (2026-11-13)"));
    assert_eq!(replies[1]["id"], 2);
    assert!(replies[1]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("超时"));

    // 只有看行情的消息时, 根本不碰闸门.
    let replies = mcp_replies(vec![market_call(3)], None, |_task| {
        runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { Err(limits::ComputeError::Timeout) }
    })
    .await;
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(replies[0]["error"]["message"], "服务端未加载数据");
}

#[tokio::test]
async fn test_mcp_lists_six_tools() {
    let replies = mcp_replies(
        vec![rpc(1, "tools/list", json!({}))],
        None,
        |task| async move { Ok(task()) },
    )
    .await;
    let names: Vec<&str> = replies[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "today_blend",
            "check_ratios",
            "find_coal",
            "get_contract",
            "market",
            "quote_blend"
        ]
    );
}

/// 两种启用的煤、一条硫上限: 询价精算能真算出配方. 今天定在 2026-11-14, 与跑测试的日子无关.
fn quote_context() -> mcp::Context {
    mcp::Context {
        master: json!({
            "default_contract": {"name": "默认合同", "specs": [
                {"indicator": "S", "direction": "Upper", "max": 1.5, "enabled": true}
            ]},
            "price_factor": {"adjusts": [
                {"indicator": "S", "direction": "Upper", "base": 1.3,
                 "premium_rate": 75, "discount_rate": 125}
            ]},
            "coals": [
                {"name": "低硫", "status": "verified", "coal_type": "焦煤", "props": {"S": 0.6}},
                {"name": "高硫", "status": "verified", "coal_type": "瘦煤", "props": {"S": 2.4}}
            ]
        }),
        storage: database::UserStorage {
            initialized: true,
            coal_prefs: json!({}),
            contract: None,
            quantity: 1000.0,
            user_coals: json!([]),
            shovel_tons: None,
        },
        benchmarks: vec![
            json!({"name": "焦煤基准", "coal_type": "焦煤", "props": {"S": 1.3},
                                "price": 2050.0, "date": "2026-10-10"}),
        ],
        rank_k: None,
        coal_prices: None,
        market: None,
        today: "2026-11-14".parse().unwrap(),
    }
}

/// 一条询价精算: 低硫按 `fob` 报价, 高硫固定; 可带本次合同要求.
fn quote_call(id: i64, fob: f64, specs: Value) -> Value {
    rpc(
        id,
        "tools/call",
        json!({ "name": "quote_blend", "arguments": {
            "quotes": {"低硫": {"fob": fob, "frt": 60}, "高硫": {"fob": 1700, "frt": 40}},
            "specs": specs
        } }),
    )
}

fn reply_text(reply: &Value) -> &str {
    reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
}

const ARCHIVED_LINE: &str = "这批报价已按批存档, 供后台「询价对比」校准估价。";

type Stored = std::sync::Mutex<Vec<(chrono::NaiveDate, quotes::QuoteBatch)>>;

/// 走 `mcp_post` (密钥已由中间件验过) 之后的那一段: 闸门开或关, 存档按 `outcome` 回, 落库的 (日期, 批) 记进 `stored`.
async fn answer_with(
    context: mcp::Context,
    messages: Vec<Value>,
    gate_open: bool,
    outcome: Result<(), quotes::StoreError>,
    stored: &Stored,
) -> Vec<Value> {
    let run = |task: McpTask| async move {
        if gate_open {
            Ok(task())
        } else {
            Err(limits::ComputeError::Timeout)
        }
    };
    let store = |day, batch| {
        stored.lock().unwrap().push((day, batch));
        async move { outcome }
    };
    mcp_answer(messages, Some(context), run, store).await
}

/// 求解闸门拒绝 (忙、超时) 时询价精算回的是 JSON-RPC error: 没求解, 报价不得进询价表.
#[tokio::test]
async fn test_mcp_quote_refused_by_the_solver_is_not_archived() {
    let stored = Stored::default();
    let messages = vec![quote_call(1, 2200.0, json!({}))];
    // 报价本身合法: 光看消息是有一批可存的, 不存只因为没求解.
    assert_eq!(
        mcp_quote_archives(&messages, Some(&quote_context())).len(),
        1
    );
    let replies = answer_with(quote_context(), messages.clone(), false, Ok(()), &stored).await;
    assert_eq!(replies[0]["error"]["code"], -32603);
    assert!(replies[0].get("result").is_none());
    assert!(stored.lock().unwrap().is_empty(), "闸门拒绝的调用存了档");

    // 同一条消息, 闸门放行就存一批.
    let replies = answer_with(quote_context(), messages, true, Ok(()), &stored).await;
    assert_eq!(replies[0]["result"]["isError"], false);
    assert_eq!(stored.lock().unwrap().len(), 1);
}

/// 一个请求最多存档一批: 多条询价精算只存最后一条算成的 (无解也算), 其余在各自回包里说明;
/// 参数没过校验的、不回包的通知、别的工具都不存也不带存档的话.
#[tokio::test]
async fn test_mcp_archives_only_the_last_solved_quote_blend() {
    let stored = Stored::default();
    let mut notification = quote_call(0, 2250.0, json!({}));
    notification.as_object_mut().unwrap().remove("id");
    let messages = vec![
        quote_call(1, 2200.0, json!({})),
        quote_call(2, -5.0, json!({})),
        notification,
        rpc(
            3,
            "tools/call",
            json!({ "name": "today_blend", "arguments": {} }),
        ),
        quote_call(4, 2300.0, json!({})),
        quote_call(5, 2400.0, json!({"S": {"max": 0.3}})),
        quote_call(6, 0.0, json!({})),
    ];
    let replies = answer_with(quote_context(), messages, true, Ok(()), &stored).await;
    let ids: Vec<_> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(
        ids,
        [json!(1), json!(2), json!(3), json!(4), json!(5), json!(6)]
    );

    let stored = stored.lock().unwrap();
    assert_eq!(stored.len(), 1, "一个请求只落一批");
    let low = stored[0].1.items.iter().find(|item| item.name == "低硫");
    assert_eq!(low.unwrap().fob, 2400.0, "存的是最后一条算成的");

    for skipped in [&replies[0], &replies[3]] {
        let text = reply_text(skipped);
        assert_eq!(skipped["result"]["isError"], false);
        assert!(text.starts_with("询价精算 (") && text.contains("\n配方:\n"));
        assert!(
            text.ends_with("\n这批报价没有存档: 同一请求里有多条询价精算, 只存档最后一条。"),
            "{text}"
        );
        assert!(!text.contains("已按批存档"), "{text}");
    }
    let last = reply_text(&replies[4]);
    assert!(last.contains("无解"), "{last}");
    assert!(last.ends_with(&format!("\n{ARCHIVED_LINE}")), "{last}");
    for refused in [&replies[1], &replies[5]] {
        assert_eq!(refused["result"]["isError"], true);
        assert!(!reply_text(refused).contains("存档"));
    }
    assert!(!reply_text(&replies[2]).contains("存档"));
}

/// 一条算成了的询价精算, 存档按 `outcome` 回时的回包文本.
async fn quote_reply_when_store(outcome: Result<(), quotes::StoreError>) -> String {
    let stored = Stored::default();
    let messages = vec![quote_call(1, 2200.0, json!({}))];
    let replies = answer_with(quote_context(), messages, true, outcome, &stored).await;
    assert_eq!(stored.lock().unwrap().len(), 1);
    assert_eq!(replies[0]["result"]["isError"], false);
    reply_text(&replies[0]).to_string()
}

/// "已存档" 这句话在落库之后才写: 存成了说存了, 没存成 (含没有数据库) 如实说, 精算结果照常给.
#[tokio::test]
async fn test_mcp_quote_reply_says_whether_the_archive_happened() {
    let saved = quote_reply_when_store(Ok(())).await;
    assert!(saved.ends_with(&format!("\n{ARCHIVED_LINE}")), "{saved}");
    assert!(!saved.contains("没有存档"), "{saved}");

    let unavailable = Box::new(StatusCode::SERVICE_UNAVAILABLE.into_response());
    for (error, reason) in [
        (quotes::StoreError::Unavailable(unavailable), "数据库未配置"),
        // 数据库报错的原文只进日志, 回包里是简短说法.
        (
            quotes::StoreError::Database(sqlx::Error::PoolTimedOut),
            "数据库写入失败",
        ),
        (
            quotes::StoreError::Invalid("低硫 的产地不对".into()),
            "低硫 的产地不对",
        ),
    ] {
        let failed = quote_reply_when_store(Err(error)).await;
        assert!(
            failed.ends_with(&format!(
                "\n这批报价没有存档成功 (不影响上面的结果): {reason}"
            )),
            "{failed}"
        );
        assert!(!failed.contains("已按批存档"), "{failed}");
        // 除了最后一行, 结果与存成了的那次逐字相同.
        assert_eq!(
            failed.rsplit_once('\n').unwrap().0,
            saved.rsplit_once('\n').unwrap().0
        );
    }
}

/// 批号里的日期与落库的日期是同一天 (装数据时取的 "今天"), 不在落库时另取一次.
#[tokio::test]
async fn test_mcp_archive_day_is_the_day_in_the_batch_id() {
    for day in ["2026-11-14", "2026-12-31"] {
        let mut context = quote_context();
        context.today = day.parse().unwrap();
        let messages = vec![quote_call(1, 2200.0, json!({}))];
        let archives = mcp_quote_archives(&messages, Some(&context));
        assert_eq!(archives[0].day, context.today);

        let stored = Stored::default();
        answer_with(context, messages, true, Ok(()), &stored).await;
        let stored = stored.lock().unwrap();
        let (stored_day, batch) = &stored[0];
        assert_eq!(stored_day.to_string(), day);
        let prefix = format!("mcp-{}-", stored_day.format("%Y%m%d"));
        assert!(batch.batch_id.starts_with(&prefix), "{}", batch.batch_id);
        assert_eq!(batch.batch_id, archives[0].batch.batch_id);
    }
}

/// 回包按消息对应: 位置对得上、id 也相同才算; 两条消息用了同一个 id 也不会把话写到别人的回包里.
#[tokio::test]
async fn test_mcp_archive_note_goes_to_its_own_reply() {
    let stored = Stored::default();
    let messages = vec![
        rpc(
            1,
            "tools/call",
            json!({ "name": "today_blend", "arguments": {} }),
        ),
        quote_call(1, 2200.0, json!({})),
    ];
    let replies = answer_with(quote_context(), messages.clone(), true, Ok(()), &stored).await;
    assert!(!reply_text(&replies[0]).contains("存档"));
    assert!(reply_text(&replies[1]).ends_with(ARCHIVED_LINE));
    assert_eq!(stored.lock().unwrap().len(), 1);

    // 回包对不上 (缺了、那个位置上的 id 不同) 就不存.
    let other = json!({"jsonrpc": "2.0", "id": 9, "result": replies[1]["result"].clone()});
    for replies in [vec![], vec![replies[0].clone(), other]] {
        let archives = mcp_quote_archives(&messages, Some(&quote_context()));
        assert_eq!(archives.len(), 1);
        let (last, skipped) = mcp_archives_to_store(archives, &replies);
        assert!(last.is_none() && skipped.is_empty());
    }
}

#[test]
fn test_mcp_reply_with_line_appends_to_the_tool_text() {
    let reply = json!({"jsonrpc": "2.0", "id": 7, "result": {
        "content": [{"type": "text", "text": "第一行\n第二行"}], "isError": false
    }});
    let noted = mcp_reply_with_line(reply.clone(), "补的一行");
    assert_eq!(reply_text(&noted), "第一行\n第二行\n补的一行");
    // 只动文本, 别的字段原样.
    let mut expected = reply;
    expected["result"]["content"][0]["text"] = json!("第一行\n第二行\n补的一行");
    assert_eq!(noted, expected);

    // 没有文本可补的回包 (JSON-RPC error、握手) 原样返回.
    for reply in [
        json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32603, "message": "求解超时"}}),
        json!({"jsonrpc": "2.0", "id": 7, "result": {"tools": []}}),
        json!({"jsonrpc": "2.0", "id": 7, "result": {"content": []}}),
    ] {
        assert_eq!(mcp_reply_with_line(reply.clone(), "补的一行"), reply);
    }
}

#[test]
fn test_mcp_archive_line_states_the_outcome() {
    assert_eq!(mcp_archive_line(&Ok(())), ARCHIVED_LINE);
    assert_eq!(
        mcp_archive_line(&Err(quotes::StoreError::Database(sqlx::Error::PoolClosed))),
        "这批报价没有存档成功 (不影响上面的结果): 数据库写入失败"
    );
}

/// 存档规则穷举时一条消息是什么.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sent {
    /// 报价合法的询价精算: 光看消息有一批可存.
    Quote,
    /// 报价写错的询价精算 (没求解, 没有可存的).
    BadQuote,
    /// 别的工具.
    Other,
    /// 不带 id 的询价精算: 通知, 不回包.
    QuoteNotification,
}

/// 那条消息的回包是哪一种.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Got {
    /// 工具结果, `isError == false`: 求解过.
    Solved,
    /// 工具结果, `isError == true`.
    ToolError,
    /// JSON-RPC error (求解闸门拒绝、超时).
    RpcError,
}

/// 一条消息可能的 (消息, 回包). 写错的报价不会有 "算成了" 的回包; 通知没有回包.
const ARCHIVE_CASES: [(Sent, Option<Got>); 9] = [
    (Sent::Quote, Some(Got::Solved)),
    (Sent::Quote, Some(Got::ToolError)),
    (Sent::Quote, Some(Got::RpcError)),
    (Sent::BadQuote, Some(Got::ToolError)),
    (Sent::BadQuote, Some(Got::RpcError)),
    (Sent::Other, Some(Got::Solved)),
    (Sent::Other, Some(Got::ToolError)),
    (Sent::Other, Some(Got::RpcError)),
    (Sent::QuoteNotification, None),
];

/// 第 `at` 条消息的出厂报价: 每条不同, 存下来的那一批是哪条消息的一看便知.
fn case_fob(at: usize) -> f64 {
    2100.0 + 100.0 * at as f64
}

fn case_message(at: usize, sent: Sent) -> Value {
    let id = at as i64 + 1;
    match sent {
        Sent::Quote => quote_call(id, case_fob(at), json!({})),
        Sent::BadQuote => quote_call(id, -5.0, json!({})),
        Sent::Other => rpc(
            id,
            "tools/call",
            json!({ "name": "today_blend", "arguments": {} }),
        ),
        Sent::QuoteNotification => {
            let mut message = quote_call(id, case_fob(at), json!({}));
            message.as_object_mut().unwrap().remove("id");
            message
        }
    }
}

fn case_reply(at: usize, got: Got) -> Value {
    let id = at as i64 + 1;
    let result = |is_error: bool| {
        json!({"jsonrpc": "2.0", "id": id, "result": {
            "content": [{"type": "text", "text": format!("第 {id} 条的正文")}],
            "isError": is_error
        }})
    };
    match got {
        Got::Solved => result(false),
        Got::ToolError => result(true),
        Got::RpcError => json!({"jsonrpc": "2.0", "id": id,
                                "error": {"code": -32603, "message": "求解超时"}}),
    }
}

/// 3 条消息以内 (消息, 回包) 的全部组合.
fn archive_combinations() -> Vec<Vec<(Sent, Option<Got>)>> {
    let mut all = Vec::new();
    for len in 1..=3_u32 {
        for code in 0..ARCHIVE_CASES.len().pow(len) {
            let combination = (0..len)
                .map(|digit| {
                    ARCHIVE_CASES[code / ARCHIVE_CASES.len().pow(digit) % ARCHIVE_CASES.len()]
                })
                .collect();
            all.push(combination);
        }
    }
    all
}

/// 询价存档规则, 对 3 条消息以内的全部组合逐一核对 (回包是照组合造的, 不真求解):
/// - 只有 "报价合法的询价精算 + 回包是没报错的工具结果" 才算求解过; 一个请求最多落一批, 是最后一条;
/// - 落库的那条回包结尾是存档的成败 (存成了说存了, 没存成说没存成), 其余算成了的结尾是
///   "只存档最后一条", 这两句话都必须为真;
/// - 其余回包 (工具报错、JSON-RPC error、别的工具) 一个字不动; 回包条数不变.
#[tokio::test]
async fn test_mcp_archive_rules_hold_for_every_combination_up_to_three_messages() {
    const NOT_LAST: &str = "这批报价没有存档: 同一请求里有多条询价精算, 只存档最后一条。";
    let context = quote_context();
    let combinations = archive_combinations();
    assert_eq!(combinations.len(), 9 + 81 + 729);
    for combination in combinations {
        for store_succeeds in [true, false] {
            let messages: Vec<Value> = combination
                .iter()
                .enumerate()
                .map(|(at, (sent, _))| case_message(at, *sent))
                .collect();
            // 回包只有带 id 的消息才有, 顺序同消息; 记下每条回包是第几条消息的.
            let answered: Vec<(usize, Sent, Got)> = combination
                .iter()
                .enumerate()
                .filter_map(|(at, (sent, got))| Some((at, *sent, (*got)?)))
                .collect();
            let before: Vec<Value> = answered
                .iter()
                .map(|(at, _, got)| case_reply(*at, *got))
                .collect();
            let solved: Vec<usize> = answered
                .iter()
                .enumerate()
                .filter(|(_, (_, sent, got))| *sent == Sent::Quote && *got == Got::Solved)
                .map(|(reply_at, _)| reply_at)
                .collect();
            let label = format!("{combination:?}, 存档成功 = {store_succeeds}");

            // 决策本身 (纯函数): 存最后一条算成的, 其余算成的另列.
            let archives = mcp_quote_archives(&messages, Some(&context));
            let (last, skipped) = mcp_archives_to_store(archives, &before);
            assert_eq!(
                last.as_ref().map(|archive| archive.reply_at),
                solved.last().copied(),
                "{label}"
            );
            let skipped: Vec<usize> = skipped.iter().map(|archive| archive.reply_at).collect();
            assert_eq!(skipped, solved[..solved.len().saturating_sub(1)], "{label}");

            // 落库与回包结尾的话.
            let stored = Stored::default();
            let store = |day, batch| {
                stored.lock().unwrap().push((day, batch));
                let outcome = if store_succeeds {
                    Ok(())
                } else {
                    Err(quotes::StoreError::Invalid("库里放不下".into()))
                };
                async move { outcome }
            };
            let mut after = before.clone();
            let archives = mcp_quote_archives(&messages, Some(&context));
            archive_mcp_quotes(archives, &mut after, store).await;

            let stored = stored.lock().unwrap();
            assert_eq!(stored.len(), solved.len().min(1), "{label}");
            assert_eq!(after.len(), before.len(), "{label}");
            for (reply_at, (reply, original)) in after.iter().zip(&before).enumerate() {
                let Some(position) = solved.iter().position(|at| *at == reply_at) else {
                    assert_eq!(reply, original, "{label}: 第 {reply_at} 条回包不该动");
                    continue;
                };
                let is_last = position + 1 == solved.len();
                let line = match (is_last, store_succeeds) {
                    (false, _) => NOT_LAST.to_string(),
                    (true, true) => ARCHIVED_LINE.to_string(),
                    (true, false) => {
                        "这批报价没有存档成功 (不影响上面的结果): 库里放不下".to_string()
                    }
                };
                let mut expected = original.clone();
                expected["result"]["content"][0]["text"] =
                    json!(format!("{}\n{line}", reply_text(original)));
                assert_eq!(reply, &expected, "{label}: 第 {reply_at} 条回包");
                if is_last {
                    // 落库的就是这条消息的那批报价, 日期是装数据时的 "今天".
                    let message_at = answered[reply_at].0;
                    let (day, batch) = &stored[0];
                    assert_eq!(*day, context.today, "{label}");
                    let low = batch.items.iter().find(|item| item.name == "低硫");
                    assert_eq!(
                        low.map(|item| item.fob),
                        Some(case_fob(message_at)),
                        "{label}"
                    );
                }
            }
        }
    }
}
