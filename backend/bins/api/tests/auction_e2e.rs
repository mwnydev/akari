//! 拍卖端到端：本机 anvil + PostgreSQL + Valkey，forum-api、forum-sponsor、forum-worker 结算与 forum-indexer 都是真实的。
//!
//! 部署与 forum-sponsor 测试相同的一套合约（`../../sponsor/tests/common`，EntryPoint、测试用 Kernel 账户、MockUSDT、
//! EscrowConfig（拍卖签名者 = forum-sponsor 的私钥）、Escrow、FixedFeePaymaster），再装上 Solady 的 ERC-6492 验证器。
//! 1. 卖家的钱包还没部署：用 ERC-6492 包装的签名签人民币拍卖授权，forum-api 经链上无部署调用验签（签错的被拒）；
//!    授权哈希与链上 `hashAuction` 一致。
//! 2. 两人出价、结束，forum-worker 请 forum-sponsor 签第 1 份成交证明给最高价者（证明哈希与链上 `hashProof` 一致）。
//! 3. 中标者不付款：付款期过后记违约、自动禁拍，次高者拿到第 2 份证明；旧证明不再被代付。
//! 4. 次高者经 forum-sponsor 代付调用 `createFromAuction` 付款建单，合约按卖家汇率换算的 USDT 与接口给的一致；
//!    forum-indexer 看到 `AuctionSettled` 后把拍卖标为已成交。
//!
//! 找不到 anvil 或 forge，或没有 `VALKEY_URL` 时跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

#[path = "../../sponsor/tests/common/mod.rs"]
mod chain;
mod common;

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::providers::Provider;
use alloy::signers::SignerSync;
use axum::http::StatusCode;
use forum_api::routes::aa::SponsorProxy;
use forum_api::wallet::{RpcAuctionChain, WalletState};
use forum_chain::aa::UserOperation;
use forum_chain::bindings::{Escrow, MockUSDT};
use forum_chain::config::{ChainConfig, Contracts, Endpoint, IndexerPolicy, RpcConfig, Token};
use forum_chain::rpc::FailoverRpc;
use forum_core::auction::AuctionSettings;
use forum_db::PgPool;
use forum_db::forums::{self, NewForum};
use forum_sponsor::kernel;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn core_address(a: Address) -> forum_core::eip712::Address {
    forum_core::eip712::Address(a.into_array())
}

fn num(v: &Value) -> U256 {
    v.as_str().unwrap().parse().unwrap()
}

/// typed data 的 `message` → 合约调用的结构。
fn onchain_auth(m: &Value) -> Escrow::AuctionAuth {
    Escrow::AuctionAuth {
        seller: m["seller"].as_str().unwrap().parse().unwrap(),
        token: m["token"].as_str().unwrap().parse().unwrap(),
        auctionId: num(&m["auctionId"]),
        reservePrice: num(&m["reservePrice"]),
        currency: m["currency"].as_u64().unwrap() as u8,
        cnyPerUsdt: m["cnyPerUsdt"].as_u64().unwrap() as u32,
        feePayer: m["feePayer"].as_u64().unwrap() as u8,
        maxFeeBps: m["maxFeeBps"].as_u64().unwrap() as u16,
        maxBaseGas: num(&m["maxBaseGas"]),
        maxMinFee: num(&m["maxMinFee"]),
        instant: m["instant"].as_bool().unwrap(),
        shipWindow: m["shipWindow"].as_u64().unwrap() as u32,
        confirmWindow: m["confirmWindow"].as_u64().unwrap() as u32,
        listingHash: m["listingHash"].as_str().unwrap().parse().unwrap(),
        expiry: m["expiry"].as_u64().unwrap(),
    }
}

fn onchain_proof(p: &Value) -> Escrow::SettlementProof {
    Escrow::SettlementProof {
        authHash: p["authHash"].as_str().unwrap().parse().unwrap(),
        winner: p["winner"].as_str().unwrap().parse().unwrap(),
        price: num(&p["price"]),
        payDeadline: p["payDeadline"].as_u64().unwrap(),
    }
}

fn hex(b: &[u8]) -> String {
    format!("0x{}", ::hex::encode(b))
}

async fn eventually<T, F, Fut>(what: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    for _ in 0..300 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

/// 已开通交易的用户，钱包是 `salt` 对应的（未部署）测试账户，里面先打 `usdt`。
struct Trader {
    id: i64,
    cookies: Vec<String>,
    owner: alloy::signers::local::PrivateKeySigner,
    wallet: Address,
    factory_data: Vec<u8>,
}

async fn trader(app: &axum::Router, db: &PgPool, w: &chain::World, salt: u8, usdt: u64) -> Trader {
    let (owner, wallet, factory_data) = chain::wallet(w, B256::repeat_byte(salt), usdt).await;
    let (id, cookies) = common::user(app, db).await;
    sqlx::query("UPDATE users SET created_at = now() - interval '30 days' WHERE id = $1")
        .bind(id)
        .execute(db)
        .await
        .unwrap();
    common::enable_trading_on(
        app,
        db,
        id,
        &cookies,
        &hex(wallet.as_slice()),
        chain::CHAIN_ID,
    )
    .await;
    Trader {
        id,
        cookies,
        owner,
        wallet,
        factory_data,
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn auction_settles_on_chain_through_the_sponsor(db: PgPool) {
    let (Some(anvil_bin), Some(forge)) = (chain::foundry_bin("anvil"), chain::foundry_bin("forge"))
    else {
        return;
    };
    if forum_testkit::valkey_url().is_none() {
        return;
    }
    let out = chain::compile_fixtures(&forge);
    let anvil = chain::Anvil::spawn(&anvil_bin);
    let w = chain::World::deploy(&anvil.http(), &out).await;
    chain::deploy_erc6492_verifier(&w).await;

    // forum-sponsor（本机端口）、forum-indexer、forum-api。
    let sponsor = chain::launch(&db, &w, anvil.http()).await;
    let shutdown = CancellationToken::new();
    let mut tasks = sponsor.spawn_background(&shutdown);
    let sponsor_router = sponsor.router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sponsor_url = format!("http://{}", listener.local_addr().unwrap());
    {
        let router = sponsor_router.clone();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    }
    let chain_cfg = ChainConfig {
        chain_id: chain::CHAIN_ID,
        name: "anvil".into(),
        rpc: RpcConfig {
            primary: Endpoint {
                http: anvil.http(),
                ws: Some(anvil.ws()),
            },
            backup: None,
        },
        contracts: Contracts {
            escrow: w.escrow,
            stream: Address::repeat_byte(0x02),
            config: w.config,
            start_block: 0,
        },
        token: Token {
            symbol: "mUSDT".into(),
            address: w.usdt,
            decimals: 6,
        },
        confirmation: Default::default(),
        indexer: IndexerPolicy {
            reconcile_interval_ms: 300,
            ..IndexerPolicy::default()
        },
        wallet_init_versions: Vec::new(),
        aa: None,
    };
    let indexer = tokio::spawn(forum_indexer::run(
        db.clone(),
        chain_cfg.clone(),
        None,
        shutdown.clone(),
    ));
    let rpc = Arc::new(FailoverRpc::from_config(&chain_cfg).unwrap());
    let wallet = WalletState {
        chain: forum_api::config::ChainConfig {
            chain_id: chain::CHAIN_ID,
            escrow: Some(core_address(w.escrow)),
            usdt: Some(core_address(w.usdt)),
            wallet_init_versions: vec![forum_chain::anvil::dev_wallet_version()],
            contracts: vec![core_address(w.escrow), core_address(w.usdt)],
        },
        rpc: Some(rpc.clone()),
        auction_chain: Arc::new(RpcAuctionChain {
            rpc: rpc.clone(),
            token: core_address(w.usdt),
        }),
        sponsor: Some(SponsorProxy::new(&sponsor_url, chain::TOKEN.into()).unwrap()),
        ..common::wallet_state()
    };
    let app = common::build_with(db.clone(), wallet).unwrap();
    forums::create(
        &db,
        &NewForum {
            parent_id: None,
            slug: "market",
            name: "market",
            description: "",
            position: 0,
            view_min_level: 0,
            post_min_level: 1,
            reply_min_level: 1,
            allow_trade: true,
            allow_auction: true,
            color: None,
        },
    )
    .await
    .unwrap();

    let seller = trader(&app, &db, &w, 1, 0).await;
    let bidder_a = trader(&app, &db, &w, 2, 200 * chain::U).await;
    let bidder_b = trader(&app, &db, &w, 3, 200 * chain::U).await;
    forum_db::settings::set_auction(
        &db,
        &AuctionSettings {
            enabled: true,
            ..AuctionSettings::default()
        },
        seller.id,
    )
    .await
    .unwrap();

    // 1. 人民币拍卖，卖家汇率 7.2：起拍 50 元、幅度 10 元、保留价 60 元；卖家承担手续费。
    let r = common::call(
        &app,
        "POST",
        "/api/v1/forums/market/auctions",
        Some(json!({
            "title": "古董怀表", "html": "<p>走时准确</p>", "source": {"type": "doc"},
            "currency": "cny", "cny_per_usdt": 72_000,
            "start_price": "5000", "increment": "1000", "reserve_price": "6000",
            "duration_secs": 86_400,
            "eligibility": {"kind": "percent", "percent": 50},
            "terms": {"ship_window": 3 * 86_400, "confirm_window": 7 * 86_400}
        })),
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    let id = r.body["id"].as_i64().unwrap();
    let r = common::call(
        &app,
        "POST",
        &format!("/api/v1/auctions/{id}/start"),
        None,
        &seller.cookies,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    let message = r.body["typed_data"]["message"].clone();
    let auth_hash: B256 = r.body["auth_hash"].as_str().unwrap().parse().unwrap();
    let escrow = Escrow::new(w.escrow, &w.p);
    assert_eq!(
        escrow
            .hashAuction(onchain_auth(&message))
            .call()
            .await
            .unwrap(),
        auth_hash,
        "forum-core hashAuction matches Solidity"
    );
    let finish = |sig: Vec<u8>| {
        let (app, c) = (app.clone(), seller.cookies.clone());
        async move {
            common::call(
                &app,
                "POST",
                &format!("/api/v1/auctions/{id}/finish"),
                Some(json!({"signature": hex(&sig)})),
                &c,
            )
            .await
        }
    };
    let inner = seller.owner.sign_hash_sync(&auth_hash).unwrap().as_bytes();
    let forged = alloy::signers::local::PrivateKeySigner::random()
        .sign_hash_sync(&auth_hash)
        .unwrap()
        .as_bytes();
    let r = finish(chain::erc6492_wrap(
        w.factory,
        &seller.factory_data,
        &forged,
    ))
    .await;
    assert_eq!(r.body["code"], "AUCTION_SIGNATURE_INVALID", "{:?}", r.body);
    let seller_sig = chain::erc6492_wrap(w.factory, &seller.factory_data, &inner);
    let r = finish(seller_sig.clone()).await;
    assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
    assert_eq!(r.body["state"], "live");
    assert!(
        w.p.get_code_at(seller.wallet).await.unwrap().is_empty(),
        "verification does not deploy the seller's wallet"
    );

    // 2. 出价：A 60 元（刚好保留价），B 70 元。钱包里各有 200 USDT，满足「余额 ≥ 出价的 50%」。
    let bid = |p: &Trader, amount: &str| {
        let (app, c) = (app.clone(), p.cookies.clone());
        let amount = amount.to_owned();
        async move {
            common::call(
                &app,
                "POST",
                &format!("/api/v1/auctions/{id}/bids"),
                Some(json!({"amount": amount})),
                &c,
            )
            .await
        }
    };
    assert_eq!(bid(&bidder_a, "6000").await.status, StatusCode::CREATED);
    let r = bid(&bidder_b, "7000").await;
    assert_eq!(r.status, StatusCode::CREATED, "{:?}", r.body);
    assert_eq!(r.body["reserve_met"], true);
    sqlx::query("UPDATE auctions SET ends_at = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(&db)
        .await
        .unwrap();

    let live = forum_worker::auctions::Live::new(
        FailoverRpc::from_config(&chain_cfg).unwrap(),
        w.escrow,
        w.usdt,
        &sponsor_url,
        chain::TOKEN.into(),
    )
    .unwrap();
    let no_grace = forum_worker::auctions::Config {
        grace: time::Duration::ZERO,
    };
    forum_worker::auctions::run_once(&db, &live, no_grace, None)
        .await
        .unwrap();
    let payment = |p: &Trader| {
        let (app, c) = (app.clone(), p.cookies.clone());
        async move {
            common::call(
                &app,
                "GET",
                &format!("/api/v1/auctions/{id}/payment"),
                None,
                &c,
            )
            .await
        }
    };
    let pay_b = payment(&bidder_b).await;
    assert_eq!(pay_b.status, StatusCode::OK, "{:?}", pay_b.body);
    assert_eq!(pay_b.body["round"], 1);
    assert_eq!(
        pay_b.body["proof"]["winner"],
        hex(bidder_b.wallet.as_slice())
    );
    assert_eq!(payment(&bidder_a).await.status, StatusCode::FORBIDDEN);
    // 证明哈希与链上 hashProof 一致，签名者是链上登记的拍卖签名者。
    let proof_b = onchain_proof(&pay_b.body["proof"]);
    let digest = escrow.hashProof(proof_b.clone()).call().await.unwrap();
    let proof_core: forum_core::eip712::SettlementProof =
        serde_json::from_value(pay_b.body["proof"].clone()).unwrap();
    let domain = forum_core::eip712::Domain {
        chain_id: chain::CHAIN_ID,
        verifying_contract: core_address(w.escrow),
    };
    assert_eq!(proof_core.hash(&domain).0, digest.0);
    let platform_sig: Bytes = pay_b.body["platform_signature"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let sig = alloy::primitives::Signature::try_from(platform_sig.as_ref()).unwrap();
    assert_eq!(
        sig.recover_address_from_prehash(&digest).unwrap(),
        w.auction_signer.address()
    );
    // 同一轮再请求证明：forum-sponsor 拒绝发给别人。
    let refused = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("{sponsor_url}/auction/proof"))
        .bearer_auth(chain::TOKEN)
        .json(&json!({"auction_id": id, "round": 2, "bid_id": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::CONFLICT);

    // 3. B 不付款。付款期过后（这里把截止时间改到过去）：违约、禁拍，A 递补。
    for sql in [
        "UPDATE auction_proofs SET pay_deadline = now() - interval '1 second' WHERE auction_id = $1",
        "UPDATE auctions SET pay_deadline = now() - interval '1 second' WHERE id = $1",
    ] {
        sqlx::query(sql).bind(id).execute(&db).await.unwrap();
    }
    forum_worker::auctions::run_once(&db, &live, no_grace, None)
        .await
        .unwrap();
    assert_eq!(
        forum_db::sanctions::active_kind(
            &db,
            bidder_b.id,
            &[forum_db::sanctions::KIND_AUCTION_BAN],
            None
        )
        .await
        .unwrap(),
        Some(forum_db::sanctions::KIND_AUCTION_BAN)
    );
    let pay_a = payment(&bidder_a).await;
    assert_eq!(pay_a.status, StatusCode::OK, "{:?}", pay_a.body);
    assert_eq!(pay_a.body["round"], 2);
    assert_eq!(pay_a.body["price"], "6000");
    // 60 元 ÷ 7.2 = 8.333333… USDT，向上取整
    assert_eq!(pay_a.body["amount"], "8333334");

    let ep = json!(w.entry_point);
    let chain_hex = json!(format!("{:#x}", chain::CHAIN_ID));
    let auth = onchain_auth(&pay_a.body["auth"]);
    let call_data = |p: &Value, buyer_pays: U256| {
        kernel::encode(&[
            chain::usdt_call(
                w.usdt,
                MockUSDT::approveCall {
                    spender: w.escrow,
                    value: buyer_pays,
                },
            ),
            chain::usdt_call(
                w.escrow,
                Escrow::createFromAuctionCall {
                    auth: auth.clone(),
                    sellerSig: seller_sig.clone().into(),
                    proof: onchain_proof(&p["proof"]),
                    platformSig: p["platform_signature"].as_str().unwrap().parse().unwrap(),
                },
            ),
        ])
    };
    let op_for = |t: &Trader, p: &Value| UserOperation {
        sender: t.wallet,
        nonce: U256::ZERO,
        factory: Some(w.factory),
        factory_data: Some(t.factory_data.clone().into()),
        call_data: call_data(p, num(&p["buyer_pays"])),
        ..UserOperation::default()
    };
    // 旧证明（B 的第 1 轮）不再代付；A 用 B 的证明也不行。
    for (t, p) in [(&bidder_b, &pay_b.body), (&bidder_a, &pay_b.body)] {
        assert_eq!(
            chain::err_code(
                &sponsor_router,
                t.id,
                "pm_getPaymasterStubData",
                json!([op_for(t, p), ep, chain_hex, {}])
            )
            .await,
            "SPONSOR_NOT_ALLOWED"
        );
    }

    // 4. A 经 forum-sponsor 代付付款建单。
    let op = chain::prepare(
        &sponsor_router,
        bidder_a.id,
        &w,
        op_for(&bidder_a, &pay_a.body),
        &bidder_a.owner,
    )
    .await;
    let hash = chain::ok(
        &sponsor_router,
        bidder_a.id,
        "eth_sendUserOperation",
        json!([op, ep]),
    )
    .await;
    let receipt = chain::receipt(&sponsor_router, bidder_a.id, &hash).await;
    assert_eq!(receipt["success"], true, "{receipt}");
    let key = forum_chain::auction::settled_key(seller.wallet, U256::from(id));
    assert!(escrow.auctionSettled(key).call().await.unwrap());

    let (state, order_id) = eventually("auction marked settled", || async {
        let (state, order_id): (i16, Option<i64>) =
            sqlx::query_as("SELECT state, order_id FROM auctions WHERE id = $1")
                .bind(id)
                .fetch_one(&db)
                .await
                .unwrap();
        (state == forum_db::auctions::STATE_SETTLED).then_some((state, order_id))
    })
    .await;
    assert_eq!(state, forum_db::auctions::STATE_SETTLED);
    let (amount, price, buyer): (String, String, Option<i64>) = sqlx::query_as(
        "SELECT amount::text, auction_price::text, buyer_id FROM escrow_orders WHERE id = $1",
    )
    .bind(order_id.unwrap())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(amount, "8333334", "contract conversion matches the API");
    assert_eq!(price, "6000");
    assert_eq!(buyer, Some(bidder_a.id));
    let r = common::call(
        &app,
        "GET",
        &format!("/api/v1/auctions/{id}"),
        None,
        &bidder_a.cookies,
    )
    .await;
    assert_eq!(r.body["state"], "settled");
    assert_eq!(r.body["order_id"], json!(order_id));

    shutdown.cancel();
    indexer.await.unwrap().unwrap();
    for t in tasks.drain(..) {
        t.await.unwrap();
    }
}
