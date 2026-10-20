//! 端到端：本机 anvil + PostgreSQL 上真实地代付并代发 UserOperation。
//!
//! 部署 EntryPoint v0.7（`contracts/lib/account-abstraction`，forge 现场编译）、MockUSDT、EscrowConfig、Escrow、
//! FixedFeePaymaster，以及 `fixtures/TestKernelAccount.sol`（ECDSA 所有者、Kernel 同款 `execute` 入口）。
//! 经 forum-sponsor 的 HTTP 接口走完 估算 → 代付签名 → 用户签名 → 提交 → 回执：
//! 1. 首笔操作带 initCode 部署钱包，approve + cancelOffer：收 0.05 USDT，操作生效；
//! 2. 提回交易所：先用掉开通送的 1 次免费代付提币（同一 nonce 重签不多扣），签名后余额被转走、callData 在链上回滚：
//!    照样收费，记一次链上失败，提币次数不退；被取代的旧签名过期后退回，之后没有次数再提回被拒。
//!
//! 以及：别人的钱包、白名单外的调用、待上链时换 nonce 都被拒绝；余额告警写入管理员群告警且去重。
//! 另有一组经「Nitro 替身」代理连 anvil（`eth_simulateV1` 一律崩溃）：签前模拟改走 EntryPoint `eth_call`，
//! 正常操作照样代付上链，执行会回滚或扣不到代付费的操作拒签。
//!
//! 需要 `DATABASE_URL`。找不到 anvil 或 forge（`~/.foundry/bin` 或 PATH）时跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::providers::Provider;
use axum::Router;
use forum_chain::aa::{SponsorTerms, UserOperation};
use forum_chain::bindings::{Escrow, FixedFeePaymaster, MockUSDT};
use forum_db::PgPool;
use forum_sponsor::config::AlertConfig;
use forum_sponsor::kernel;
use forum_sponsor::sim::MethodCounts;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use common::*;

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn sponsors_and_relays_user_operations(db: PgPool) {
    let (Some(anvil_bin), Some(forge)) = (foundry_bin("anvil"), foundry_bin("forge")) else {
        return;
    };
    forum_runtime::init_tracing();
    let out = compile_fixtures(&forge);
    let anvil = Anvil::spawn(&anvil_bin);
    let w = World::deploy(&anvil.http(), &out).await;

    let app = launch(&db, &w, anvil.http()).await;
    let shutdown = CancellationToken::new();
    let tasks = app.spawn_background(&shutdown);
    let router = app.router();

    // 用户 A 的钱包：尚未部署的 counterfactual 地址，先往里打 10 USDT。
    let (owner, sender, factory_data) = wallet(&w, B256::ZERO, 10 * U).await;
    let user_a = new_user(&db, Some(sender)).await;
    let user_b = new_user(&db, Some(Address::repeat_byte(0xbb))).await;

    // 1. 首笔：部署钱包 + approve(escrow) + cancelOffer —— 付款前操作，收代付费。
    let call_data = kernel::encode(&[
        usdt_call(
            w.usdt,
            MockUSDT::approveCall {
                spender: w.escrow,
                value: U256::from(U),
            },
        ),
        usdt_call(
            w.escrow,
            Escrow::cancelOfferCall {
                nonce: U256::from(42),
            },
        ),
    ]);
    let op = UserOperation {
        sender,
        nonce: U256::ZERO,
        factory: Some(w.factory),
        factory_data: Some(factory_data.into()),
        call_data,
        ..UserOperation::default()
    };
    let ep = json!(w.entry_point);
    let chain = json!(format!("{CHAIN_ID:#x}"));

    // 别人的钱包、白名单外的调用都拒绝。
    assert_eq!(
        err_code(
            &router,
            user_b,
            "pm_getPaymasterStubData",
            json!([op, ep, chain, {}])
        )
        .await,
        "SPONSOR_NOT_ALLOWED"
    );
    let to_paymaster = UserOperation {
        call_data: kernel::encode(&[usdt_call(w.paymaster, FixedFeePaymaster::sweepFeesCall {})]),
        ..op.clone()
    };
    assert_eq!(
        err_code(
            &router,
            user_a,
            "pm_getPaymasterStubData",
            json!([to_paymaster, ep, chain, {}])
        )
        .await,
        "SPONSOR_NOT_ALLOWED"
    );

    let first = prepare(&router, user_a, &w, op.clone(), &owner).await;
    assert_eq!(
        SponsorTerms::decode(first.paymaster_data.as_ref().unwrap())
            .unwrap()
            .fee,
        u128::from(FEE)
    );
    let terms = SponsorTerms::decode(first.paymaster_data.as_ref().unwrap()).unwrap();
    assert_eq!(terms.valid_until - terms.valid_after, 120);

    // 签了还没提交：换一个 nonce 再要代付被拒（每个账户只有一笔待上链）；同一 nonce 可以重签。
    let other_nonce = UserOperation {
        nonce: U256::from(1),
        ..first.clone()
    };
    assert_eq!(
        err_code(
            &router,
            user_a,
            "pm_getPaymasterData",
            json!([other_nonce, ep, chain, {}])
        )
        .await,
        "SPONSOR_PENDING"
    );
    tokio::time::sleep(Duration::from_millis(1100)).await; // 换一秒，签名条款才不同
    let resigned = prepare(&router, user_a, &w, op.clone(), &owner).await;
    let old_hash = json!(first.hash(w.entry_point, CHAIN_ID).unwrap());
    assert_eq!(status(&db, &old_hash).await, "replaced");
    assert_eq!(
        err_code(&router, user_a, "eth_sendUserOperation", json!([first, ep])).await,
        "SPONSOR_NOT_ALLOWED",
        "a replaced sponsorship cannot be submitted"
    );

    let hash = ok(
        &router,
        user_a,
        "eth_sendUserOperation",
        json!([resigned, ep]),
    )
    .await;
    let included_hash = hash_of(&hash);
    let r = receipt(&router, user_a, &hash).await;
    assert_eq!(r["success"], true, "{r}");
    assert_eq!(r["sender"], json!(sender));
    assert_eq!(status(&db, &hash).await, "included");
    assert!(
        !w.p.get_code_at(sender).await.unwrap().is_empty(),
        "wallet deployed"
    );
    assert_eq!(
        w.usdt_balance(w.paymaster).await,
        U256::from(FEE),
        "fee pulled to the paymaster"
    );
    assert_eq!(w.usdt_balance(sender).await, U256::from(10 * U - FEE));
    let cancelled = Escrow::new(w.escrow, &w.p)
        .cancelledNonce(sender, U256::from(42))
        .call()
        .await
        .unwrap();
    assert!(cancelled, "callData executed");
    let by_hash = ok(&router, user_a, "eth_getUserOperationByHash", json!([hash])).await;
    assert_eq!(by_hash["transactionHash"], r["receipt"]["transactionHash"]);

    // 2. 提回交易所 9 USDT（转到已生效的提币地址）：模拟通过并签名后，用户（所有者直接调用账户）先把钱转走，链上 callData 回滚，
    //    代付费照样在验证阶段扣走。
    let exchange = Address::repeat_byte(0xe1);
    sqlx::query(
        "INSERT INTO withdraw_addresses (user_id, chain_id, address, label, active_at) VALUES ($1, $2, $3, 'ex', now())",
    )
    .bind(user_a)
    .bind(CHAIN_ID as i64)
    .bind(exchange.to_vec())
    .execute(&db)
    .await
    .unwrap();
    let transfer = UserOperation {
        sender,
        nonce: U256::from(1),
        call_data: kernel::encode(&[usdt_call(
            w.usdt,
            MockUSDT::transferCall {
                to: exchange,
                value: U256::from(9 * U),
            },
        )]),
        ..UserOperation::default()
    };
    assert_eq!(
        err_code(
            &router,
            user_a,
            "pm_getPaymasterStubData",
            json!([transfer, ep, chain, {}])
        )
        .await,
        "WITHDRAW_CREDIT_REQUIRED",
        "no credit yet"
    );
    assert!(forum_db::credits::award_starter(&db, user_a).await.unwrap());
    let replaced_transfer = prepare(&router, user_a, &w, transfer.clone(), &owner).await;
    assert_eq!(credits(&db, user_a).await, 0);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let transfer = prepare(&router, user_a, &w, transfer, &owner).await;
    let replaced_hash = json!(replaced_transfer.hash(w.entry_point, CHAIN_ID).unwrap());
    assert_eq!(status(&db, &replaced_hash).await, "replaced");
    let drain = kernel::encode(&[usdt_call(
        w.usdt,
        MockUSDT::transferCall {
            to: Address::repeat_byte(0xd0),
            value: U256::from(9 * U),
        },
    )]);
    send(&w.p, owner.address(), sender, drain.to_vec(), U256::ZERO).await;

    let hash = ok(
        &router,
        user_a,
        "eth_sendUserOperation",
        json!([transfer, ep]),
    )
    .await;
    let r = receipt(&router, user_a, &hash).await;
    assert_eq!(r["success"], false, "{r}");
    assert_eq!(status(&db, &hash).await, "failed");
    assert_eq!(
        w.usdt_balance(w.paymaster).await,
        U256::from(2 * FEE),
        "failed op still paid the fee"
    );
    assert_eq!(w.usdt_balance(exchange).await, U256::ZERO);
    let failures: i32 =
        sqlx::query_scalar("SELECT failures FROM sponsor_accounts WHERE user_id = $1")
            .bind(user_a)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(failures, 1);
    assert_eq!(
        credits(&db, user_a).await,
        -1,
        "replaced signature still holds its credit until it expires"
    );

    // 3. 过期对账：被取代的旧签名没上链 → expired；把已上链的第一笔改回「已提交、已过期」，
    //    对账从日志里找回它的结果。
    let first_hash = hash_of(&old_hash);
    sqlx::query("UPDATE sponsorships SET status = 'submitted', valid_until = now() - interval '1 minute' WHERE user_op_hash = $1")
        .bind(included_hash.to_vec())
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sponsorships SET valid_until = now() - interval '1 minute' WHERE user_op_hash = ANY($1)",
    )
    .bind(vec![first_hash.to_vec(), hash_of(&replaced_hash).to_vec()])
    .execute(&db)
    .await
    .unwrap();
    let n = forum_sponsor::ledger::reconcile_once(&db, &app.chain, 2000)
        .await
        .unwrap();
    assert_eq!(n, 3);
    assert_eq!(status(&db, &old_hash).await, "expired");
    assert_eq!(status(&db, &replaced_hash).await, "expired");
    assert_eq!(status(&db, &json!(included_hash)).await, "included");
    // 被取代的提币签名没上链：退回 1 次；上链失败的那次不退，所以又用完了。
    assert_eq!(credits(&db, user_a).await, 0);
    let again = UserOperation {
        nonce: U256::from(2),
        ..transfer.clone()
    };
    assert_eq!(
        err_code(
            &router,
            user_a,
            "pm_getPaymasterStubData",
            json!([again, ep, chain, {}])
        )
        .await,
        "WITHDRAW_CREDIT_REQUIRED"
    );

    // 3. 余额告警：告警线调高后写入管理员群告警，再查一次不重复写。
    let high = AlertConfig {
        deposit_min: U256::from(10u128.pow(20)),
        hot_wallet_min: U256::from(10u128.pow(30)),
        ..AlertConfig::default()
    };
    for _ in 0..2 {
        forum_sponsor::alerts::check_once(
            &db,
            &app.chain,
            w.paymaster,
            &app.bundler.relayer,
            &high,
        )
        .await
        .unwrap();
    }
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT payload->>'event' FROM outbox WHERE kind = 'staff_alert' ORDER BY 1",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        events,
        [
            forum_sponsor::alerts::HOT_WALLET_LOW,
            forum_sponsor::alerts::PAYMASTER_DEPOSIT_LOW
        ]
    );
    assert!(
        !app.bundler.relayer.healthy(),
        "hot wallet below threshold marks the relayer unhealthy"
    );

    shutdown.cancel();
    for t in tasks {
        t.await.unwrap();
    }
}

/// anvil 前面的 Arbitrum Nitro 替身：`eth_simulateV1` 一律回 `-32603 method handler crashed`
/// （OffchainLabs/nitro#3372），其他请求原样转发。返回地址与收到的 `eth_simulateV1` 次数。
async fn nitro_proxy(upstream: String) -> (String, Arc<AtomicUsize>) {
    let crashes = Arc::new(AtomicUsize::new(0));
    let seen = crashes.clone();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let app = Router::new().route(
        "/",
        axum::routing::post(move |body: axum::body::Bytes| {
            let (client, upstream, seen) = (client.clone(), upstream.clone(), seen.clone());
            async move {
                let req: Value = serde_json::from_slice(&body).unwrap();
                if req["method"] == "eth_simulateV1" {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let crash = json!({"jsonrpc": "2.0", "id": req["id"],
                        "error": {"code": -32603, "message": "method handler crashed"}});
                    return crash.to_string();
                }
                let res = client
                    .post(&upstream)
                    .header("content-type", "application/json")
                    .body(body)
                    .send()
                    .await
                    .unwrap();
                res.text().await.unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, crashes)
}

/// 节点的 `eth_simulateV1` 崩溃时，签前模拟改走 EntryPoint `eth_call`：正常操作照样代付、上链成功；
/// 执行会回滚、扣不到代付费的操作都拒签。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn simulates_through_the_entry_point_when_simulate_v1_crashes(db: PgPool) {
    let (Some(anvil_bin), Some(forge)) = (foundry_bin("anvil"), foundry_bin("forge")) else {
        return;
    };
    forum_runtime::init_tracing();
    let out = compile_fixtures(&forge);
    let anvil = Anvil::spawn(&anvil_bin);
    let w = World::deploy(&anvil.http(), &out).await;
    let (rpc, crashes) = nitro_proxy(anvil.http()).await;
    let app = launch(&db, &w, rpc).await;
    let shutdown = CancellationToken::new();
    let tasks = app.spawn_background(&shutdown);
    let router = app.router();
    let ep = json!(w.entry_point);
    let chain = json!(format!("{CHAIN_ID:#x}"));

    let (owner, sender, factory_data) = wallet(&w, B256::repeat_byte(1), 10 * U).await;
    let user = new_user(&db, Some(sender)).await;

    // 1. 部署钱包 + approve + cancelOffer：估算与代付签名都经备用模拟，签出的操作上链成功、收了代付费。
    let call_data = kernel::encode(&[
        usdt_call(
            w.usdt,
            MockUSDT::approveCall {
                spender: w.escrow,
                value: U256::from(U),
            },
        ),
        usdt_call(
            w.escrow,
            Escrow::cancelOfferCall {
                nonce: U256::from(7),
            },
        ),
    ]);
    let op = UserOperation {
        sender,
        nonce: U256::ZERO,
        factory: Some(w.factory),
        factory_data: Some(factory_data.into()),
        call_data,
        ..UserOperation::default()
    };
    let signed = prepare(&router, user, &w, op, &owner).await;
    assert_eq!(
        app.bundler.sponsor.sim.counts(),
        MethodCounts {
            simulate_v1: 0,
            entry_point: 2
        }
    );
    assert_eq!(
        crashes.load(Ordering::SeqCst),
        1,
        "the endpoint is not asked for eth_simulateV1 again during the cool-down"
    );
    let hash = ok(&router, user, "eth_sendUserOperation", json!([signed, ep])).await;
    let r = receipt(&router, user, &hash).await;
    assert_eq!(r["success"], true, "{r}");
    assert_eq!(w.usdt_balance(w.paymaster).await, U256::from(FEE));
    let cancelled = Escrow::new(w.escrow, &w.p)
        .cancelledNonce(sender, U256::from(7))
        .call()
        .await
        .unwrap();
    assert!(cancelled, "callData executed");

    // 2. 执行会回滚：转出比余额还多的 USDT（到已生效的提币地址，有提币次数，规则检查都能过）。
    let exchange = Address::repeat_byte(0xe2);
    sqlx::query(
        "INSERT INTO withdraw_addresses (user_id, chain_id, address, label, active_at) VALUES ($1, $2, $3, 'ex', now())",
    )
    .bind(user)
    .bind(CHAIN_ID as i64)
    .bind(exchange.to_vec())
    .execute(&db)
    .await
    .unwrap();
    assert!(forum_db::credits::award_starter(&db, user).await.unwrap());
    let overdraw = UserOperation {
        nonce: U256::from(1),
        factory: None,
        factory_data: None,
        call_data: kernel::encode(&[usdt_call(
            w.usdt,
            MockUSDT::transferCall {
                to: exchange,
                value: U256::from(100 * U),
            },
        )]),
        paymaster: None,
        paymaster_data: None,
        signature: Bytes::new(),
        ..signed.clone()
    };
    assert_eq!(
        err_code(
            &router,
            user,
            "eth_estimateUserOperationGas",
            json!([overdraw, ep])
        )
        .await,
        "SIMULATION_FAILED"
    );
    assert_eq!(
        err_code(
            &router,
            user,
            "pm_getPaymasterData",
            json!([overdraw, ep, chain, {}])
        )
        .await,
        "SIMULATION_FAILED"
    );
    assert_eq!(
        credits(&db, user).await,
        1,
        "a refused signature takes no credit"
    );

    // 3. 扣不到代付费：所有者直接把 USDT 全部转走，Paymaster 验证阶段的 transferFrom 失败（AA33）。
    let left = w.usdt_balance(sender).await;
    let drain = kernel::encode(&[usdt_call(
        w.usdt,
        MockUSDT::transferCall {
            to: Address::repeat_byte(0xd1),
            value: left,
        },
    )]);
    send(&w.p, owner.address(), sender, drain.to_vec(), U256::ZERO).await;
    let unpaid = UserOperation {
        call_data: kernel::encode(&[usdt_call(
            w.usdt,
            MockUSDT::approveCall {
                spender: w.escrow,
                value: U256::ZERO,
            },
        )]),
        ..overdraw
    };
    assert_eq!(
        err_code(
            &router,
            user,
            "pm_getPaymasterData",
            json!([unpaid, ep, chain, {}])
        )
        .await,
        "SIMULATION_FAILED"
    );
    let signatures: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sponsorships WHERE user_id = $1")
            .bind(user)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(signatures, 1, "only the valid operation was signed");

    shutdown.cancel();
    for t in tasks {
        t.await.unwrap();
    }
}
