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
