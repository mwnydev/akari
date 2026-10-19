//! `/api/v1/chain/rpc` 与 `/api/v1/wallet/balances`：方法与调用目标白名单、Valkey 短缓存、节点错误原样转回、
//! 节点不可用。上游是本机的假 JSON-RPC 节点（经真实的 `FailoverRpc` 转发），记录收到的每个请求。
//! 需要 PostgreSQL（`DATABASE_URL`）与 Valkey（`VALKEY_URL`，未设置则跳过）。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use common::*;
use forum_chain::config::Endpoint;
use forum_chain::rpc::FailoverRpc;
use forum_db::PgPool;
use serde_json::{Value, json};

const RPC: &str = "/api/v1/chain/rpc";
const WALLET: &str = "0x1111111111111111111111111111111111111111";
const STRANGER: &str = "0x7777777777777777777777777777777777777777";

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

/// 假节点：记录请求；对 ESCROW 的 `eth_call` 回 revert，其他按方法给固定结果。
async fn fake_node() -> (String, Seen) {
    async fn rpc(State(seen): State<Seen>, Json(req): Json<Value>) -> Json<Value> {
        let method = req["method"].as_str().unwrap().to_owned();
        let params = req["params"].clone();
        seen.lock().unwrap().push((method.clone(), params.clone()));
        let result = match method.as_str() {
            "eth_blockNumber" => json!("0x10"),
            "eth_call" if params[0]["to"] == ESCROW => {
                return Json(json!({
                    "jsonrpc": "2.0", "id": req["id"],
                    "error": {"code": 3, "message": "execution reverted", "data": "0x08c379a0"},
                }));
            }
            // balanceOf = 1 USDT
            "eth_call" => json!(format!("0x{:064x}", 1_000_000)),
            "eth_getBalance" => json!("0x5"),
            "eth_getCode" => json!("0x6001"),
            "eth_getTransactionReceipt" => Value::Null,
            "eth_getBlockByNumber" => json!({"number": "0x10", "transactions": []}),
            "eth_estimateGas" => json!("0x5208"),
            other => panic!("unexpected upstream method {other}"),
        };
        Json(json!({"jsonrpc": "2.0", "id": req["id"], "result": result}))
    }
    let seen = Seen::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route("/", post(rpc)).with_state(seen.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/"), seen)
}

fn with_rpc(db: PgPool, url: &str) -> Option<Router> {
    let endpoint = Endpoint {
        http: url.to_owned(),
        ws: None,
    };
    let mut wallet = wallet_state();
    wallet.rpc = Some(Arc::new(
        FailoverRpc::new(&[&endpoint], Duration::from_secs(300)).unwrap(),
    ));
    build_with(db, wallet)
}

fn req(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 9, "method": method, "params": params})
}

async fn rpc(app: &Router, cookies: &[String], method: &str, params: Value) -> Res {
    wallet_call(app, "POST", RPC, Some(req(method, params)), cookies).await
}

fn count(seen: &Seen, method: &str) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(m, _)| m == method)
        .count()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn only_allowlisted_reads_reach_the_node(db: PgPool) {
    let (url, seen) = fake_node().await;
    let Some(app) = with_rpc(db.clone(), &url) else {
        return;
    };

    // 只接受钱包页面来源；不需要登录。
    let r = call(
        &app,
        "POST",
        RPC,
        Some(req("eth_blockNumber", json!([]))),
        &[],
    )
    .await;
    assert_eq!(r.body["code"], "ORIGIN_REJECTED");
    let r = rpc(&app, &[], "eth_blockNumber", json!([])).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body, json!({"jsonrpc": "2.0", "id": 9, "result": "0x10"}));
    // 链 ID 取自链配置，不问节点。
    let r = rpc(&app, &[], "eth_chainId", json!([])).await;
    assert_eq!(r.body["result"], format!("{CHAIN_ID:#x}"));

    let usdt_call = json!({"to": USDT, "data": "0x70a08231"});
    let rejected = [
        ("eth_sendRawTransaction", json!(["0x00"])),
        ("eth_getLogs", json!([{}])),
        ("debug_traceCall", json!([usdt_call, "latest"])),
        ("eth_chainId", json!([1])),
        // 调用目标不是本站合约
        (
            "eth_call",
            json!([{"to": STRANGER, "data": "0x"}, "latest"]),
        ),
        ("eth_estimateGas", json!([{"to": STRANGER}])),
        ("eth_call", json!([{"data": "0x"}, "latest"])),
        // 状态覆盖、不认识的字段、非字符串字段
        (
            "eth_call",
            json!([usdt_call, "latest", {STRANGER: {"balance": "0x1"}}]),
        ),
        (
            "eth_call",
            json!([{"to": USDT, "accessList": []}, "latest"]),
        ),
        ("eth_call", json!([{"to": USDT, "gas": 1}, "latest"])),
        ("eth_call", json!([usdt_call, {"blockHash": "0x00"}])),
        // 不返回完整交易
        ("eth_getBlockByNumber", json!(["latest", true])),
        ("eth_getBalance", json!(["0x12"])),
        ("eth_getTransactionReceipt", json!(["0x1234"])),
        // 钱包地址只对本人放行
        ("eth_call", json!([{"to": WALLET, "data": "0x"}, "latest"])),
    ];
    for (method, params) in rejected {
        let r = rpc(&app, &[], method, params.clone()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{method} {params}");
        assert_eq!(r.body["code"], "INVALID_PARAMETER", "{method} {params}");
    }
    let r = wallet_call(
        &app,
        "POST",
        RPC,
        Some(json!([req("eth_blockNumber", json!([]))])),
        &[],
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::BAD_REQUEST,
        "batches are not supported"
    );

    for (method, params) in [
        ("eth_call", json!([usdt_call, "latest"])),
        ("eth_estimateGas", json!([{"to": USDT, "data": "0x"}])),
        ("eth_getBalance", json!([STRANGER, "latest"])),
        ("eth_getCode", json!([STRANGER])),
        (
            "eth_getTransactionReceipt",
            json!([format!("0x{}", "ab".repeat(32))]),
        ),
        ("eth_getBlockByNumber", json!(["0x10", false])),
    ] {
        let r = rpc(&app, &[], method, params.clone()).await;
        assert_eq!(r.status, StatusCode::OK, "{method} {params}: {:?}", r.body);
        assert!(r.body.get("result").is_some(), "{method}: {:?}", r.body);
    }
    // 只有放行的请求到了节点；缺省的区块标记补成 latest。
    let methods: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(m, _)| m.clone())
        .collect();
    assert_eq!(
        methods,
        [
            "eth_blockNumber",
            "eth_call",
            "eth_estimateGas",
            "eth_getBalance",
            "eth_getCode",
            "eth_getTransactionReceipt",
            "eth_getBlockByNumber"
        ]
    );
    let code_params = seen.lock().unwrap()[4].1.clone();
    assert_eq!(code_params, json!([STRANGER, "latest"]));

    // 登录后可以调自己的钱包；别人的钱包仍不行。
    let (id, cookies) = user(&app, &db).await;
    enable_trading(&app, &db, id, &cookies, WALLET).await;
    let r = rpc(
        &app,
        &cookies,
        "eth_call",
        json!([{"to": WALLET, "data": "0x"}, "latest"]),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let (_, other) = user(&app, &db).await;
    let r = rpc(
        &app,
        &other,
        "eth_call",
        json!([{"to": WALLET, "data": "0x"}, "latest"]),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn latest_reads_are_cached_briefly_and_errors_pass_through(db: PgPool) {
    let (url, seen) = fake_node().await;
    let Some(app) = with_rpc(db.clone(), &url) else {
        return;
    };
    for _ in 0..3 {
        let r = rpc(&app, &[], "eth_blockNumber", json!([])).await;
        assert_eq!(r.body["result"], "0x10");
    }
    assert_eq!(count(&seen, "eth_blockNumber"), 1);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    rpc(&app, &[], "eth_blockNumber", json!([])).await;
    assert_eq!(count(&seen, "eth_blockNumber"), 2, "1s cache expired");

    // 地址大小写与缺省标记规范化后是同一个缓存键；指定区块号的不缓存。
    let upper = STRANGER.to_uppercase().replace("0X", "0x");
    for params in [
        json!([STRANGER, "latest"]),
        json!([upper]),
        json!([STRANGER]),
    ] {
        let r = rpc(&app, &[], "eth_getBalance", params).await;
        assert_eq!(r.body["result"], "0x5");
    }
    assert_eq!(count(&seen, "eth_getBalance"), 1);
    for _ in 0..2 {
        rpc(&app, &[], "eth_getBalance", json!([STRANGER, "0x10"])).await;
    }
    assert_eq!(count(&seen, "eth_getBalance"), 3);

    // 节点的 revert 原样转回（带 revert 数据），且不缓存。
    for _ in 0..2 {
        let r = rpc(
            &app,
            &[],
            "eth_call",
            json!([{"to": ESCROW, "data": "0x"}, "latest"]),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        assert_eq!(r.body["id"], 9);
        assert_eq!(r.body["error"]["code"], 3);
        assert_eq!(r.body["error"]["data"], "0x08c379a0");
    }
    assert_eq!(count(&seen, "eth_call"), 2);
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn wallet_balances_are_read_at_one_block_and_cached(db: PgPool) {
    let (url, seen) = fake_node().await;
    let Some(app) = with_rpc(db.clone(), &url) else {
        return;
    };
    let path = "/api/v1/wallet/balances";
    let r = call(&app, "GET", path, None, &[]).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let (id, cookies) = user(&app, &db).await;
    let r = call(&app, "GET", path, None, &cookies).await;
    assert_eq!(r.body["code"], "WALLET_REQUIRED");

    enable_trading(&app, &db, id, &cookies, WALLET).await;
    let balances = |credits: i64, left: i64| {
        json!({
            "usdt": "1000000", "eth": "5", "block": 16,
            "withdraw_credits": credits, "withdraws_left_today": left,
        })
    };
    for _ in 0..2 {
        let r = call(&app, "GET", path, None, &cookies).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        assert_eq!(
            r.body,
            balances(1, 3),
            "starter credit from enabling trading"
        );
    }
    // 代付提币用掉 1 次：链上部分仍走缓存，次数立即更新。
    sqlx::query(
        "WITH s AS (
           INSERT INTO sponsorships (user_id, chain_id, sender, nonce, user_op_hash, fee, valid_until, signed_block)
           VALUES ($1, $2, decode(repeat('11', 20), 'hex'), 0, sha256('x'), 50000, now() + interval '2 minutes', 1)
           RETURNING id
         )
         INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id) SELECT $1, -1, 'withdraw', id FROM s",
    )
    .bind(id)
    .bind(CHAIN_ID as i64)
    .execute(&db)
    .await
    .unwrap();
    let r = call(&app, "GET", path, None, &cookies).await;
    assert_eq!(r.body, balances(0, 2));
    let r = call(&app, "GET", "/api/v1/openapi.json", None, &[]).await;
    let schemas = &r.body["components"]["schemas"];
    let required = schemas["Balances"]["required"].as_array().unwrap();
    for field in ["withdraw_credits", "withdraws_left_today"] {
        assert!(required.contains(&json!(field)), "{field}");
    }
    let codes = schemas["ErrorCode"]["enum"].as_array().unwrap();
    assert!(codes.contains(&json!("WITHDRAW_CREDIT_REQUIRED")));

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "cached for 3s: {seen:?}");
    let (_, call_params) = seen.iter().find(|(m, _)| m == "eth_call").unwrap();
    assert_eq!(call_params[0]["to"], USDT);
    assert_eq!(call_params[1], "0x10", "balance read at the same block");
    let (_, balance_params) = seen.iter().find(|(m, _)| m == "eth_getBalance").unwrap();
    assert_eq!(balance_params, &json!([WALLET, "0x10"]));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn unreachable_or_unconfigured_node_is_unavailable(db: PgPool) {
    // 一个刚关闭的端口：连接被拒绝。
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/", l.local_addr().unwrap())
    };
    let Some(app) = with_rpc(db.clone(), &closed) else {
        return;
    };
    let r = rpc(&app, &[], "eth_blockNumber", json!([])).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE, "{:?}", r.body);
    assert_eq!(r.body["code"], "CHAIN_UNAVAILABLE");
    // 拒绝发生在转发之前：白名单外的请求不会变成 503。
    let r = rpc(&app, &[], "eth_sendRawTransaction", json!([])).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let Some(app) = build(db.clone()) else { return };
    let r = rpc(&app, &[], "eth_getCode", json!([STRANGER])).await;
    assert_eq!(r.body["code"], "CHAIN_UNAVAILABLE");
    let (id, cookies) = user(&app, &db).await;
    enable_trading(&app, &db, id, &cookies, WALLET).await;
    let r = call(&app, "GET", "/api/v1/wallet/balances", None, &cookies).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
}
