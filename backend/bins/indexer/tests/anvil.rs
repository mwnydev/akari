//! 端到端：本机 anvil + PostgreSQL。
//!
//! 部署 MockUSDT、EscrowConfig、Escrow、StreamEscrow（用 `crates/chain/abi` 里的字节码），走一遍 createAndFund →
//! markDelivered → release，检查订单表、事件表、发件箱与报价卡片；再用 `evm_snapshot` / `evm_revert` 制造区块回滚，
//! 检查撤销与重放；然后走一遍分期订单 createAndFund → start → claim → dispute；最后多出块，检查升级到 finalized。
//! 另一个用例检查仲裁案件随争议的回滚与重放开关，以及按后端生成的 `resolve` calldata 执行后关闭。
//! 主节点故意配置成不可达，同时验证主备切换。
//!
//! 需要 `DATABASE_URL`（sqlx::test 为每个用例建临时库）。找不到 anvil（`ANVIL_BIN`、`~/.foundry/bin/anvil`
//! 或 PATH）时跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::future::Future;
use std::time::Duration;

use alloy::primitives::{Address, B256, U256};
use alloy::providers::{DynProvider, Provider};
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolEvent;
use forum_chain::anvil::{Anvil, anvil_bin};
use forum_chain::bindings::{Escrow, EscrowConfig, MockUSDT, StreamEscrow};
use forum_chain::config::{
    ChainConfig, ConfirmationPolicy, Contracts, Endpoint, IndexerPolicy, RpcConfig, Token,
};
use forum_chain::order::Market;
use forum_chain::reorg::{BlockTag, ChainSource};
use forum_db::PgPool;
use forum_testkit::require;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// anvil 默认助记词的第 1 号账户私钥（卖家，签报价用）。
const SELLER_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const U: u64 = 1_000_000;
/// 不可达的主节点：验证主备切换。
const DEAD_HTTP: &str = "http://127.0.0.1:1";
const DEAD_WS: &str = "ws://127.0.0.1:1";

async fn eventually<T, F, Fut>(what: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    for _ in 0..200 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

struct World {
    p: DynProvider,
    usdt: Address,
    config: Address,
    escrow: Address,
    stream: Address,
    buyer: Address,
    seller: PrivateKeySigner,
    /// 合约的 `arbiter`（生产为仲裁 Safe）。
    arbiter: Address,
    start_block: u64,
}

impl World {
    async fn deploy(anvil: &Anvil) -> Self {
        let p = anvil.provider().await.unwrap();
        let accounts = p.get_accounts().await.unwrap();
        let (deployer, buyer, ops) = (accounts[0], accounts[2], accounts[3]);
        let seller: PrivateKeySigner = SELLER_KEY.parse().unwrap();
        assert_eq!(seller.address(), accounts[1]);
        let start_block = p.get_block_number().await.unwrap() + 1;

        let usdt = MockUSDT::deploy_builder(&p)
            .from(deployer)
            .deploy()
            .await
            .unwrap();
        // 与 Deploy.s.sol 相同：先算出两份担保合约的地址登记为市场，再部署 Config、Escrow、StreamEscrow。
        let nonce = p.get_transaction_count(deployer).await.unwrap();
        let (escrow_addr, stream_addr) = (deployer.create(nonce + 1), deployer.create(nonce + 2));
        let params = EscrowConfig::Params {
            feeBps: 100,
            minFee: U,
            minOrder: 5 * U,
            orderCap: 200 * U,
            totalLockedCap: 5000 * U,
            baseGas: 100_000,
            claimGas: 20_000,
            disputeMulBps: 10_000,
            resolveTimeout: 30 * 86_400,
            proposalWindow: 72 * 3600,
        };
        let config = EscrowConfig::deploy_builder(
            &p,
            ops,
            params,
            usdt,
            Address::ZERO,
            vec![escrow_addr, stream_addr],
        )
        .from(deployer)
        .deploy()
        .await
        .unwrap();
        let escrow = Escrow::deploy_builder(&p, config, ops, ops)
            .from(deployer)
            .deploy()
            .await
            .unwrap();
        assert_eq!(escrow, escrow_addr);
        let stream = StreamEscrow::deploy_builder(&p, escrow)
            .from(deployer)
            .deploy()
            .await
            .unwrap();
        assert_eq!(stream, stream_addr);

        let token = MockUSDT::new(usdt, &p);
        token
            .mint(buyer, U256::from(1000 * U))
            .from(deployer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        for market in [escrow, stream] {
            token
                .approve(market, U256::MAX)
                .from(buyer)
                .send()
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
        }
        Self {
            p,
            usdt,
            config,
            escrow,
            stream,
            buyer,
            seller,
            arbiter: ops,
            start_block,
        }
    }

    fn escrow(&self) -> Escrow::EscrowInstance<&DynProvider> {
        Escrow::new(self.escrow, &self.p)
    }

    fn stream(&self) -> StreamEscrow::StreamEscrowInstance<&DynProvider> {
        StreamEscrow::new(self.stream, &self.p)
    }

    fn offer(&self, amount: u64, nonce: u64) -> Escrow::Offer {
        Escrow::Offer {
            seller: self.seller.address(),
            buyer: Address::ZERO,
            token: self.usdt,
            amount: U256::from(amount),
            feePayer: 2, // FeeMath.Payer.Half
            maxFeeBps: 100,
            maxBaseGas: U256::from(100_000),
            maxMinFee: U256::from(U),
            bond: U256::ZERO,
            lateSlashBps: 0,
            instant: false,
            shipWindow: 86_400,
            confirmWindow: 86_400,
            listingHash: B256::repeat_byte(7),
            expiry: 4_000_000_000,
            maxFills: 1,
            nonce: U256::from(nonce),
        }
    }

    /// 买家按卖家签名的报价下单付款，返回交易哈希。
    async fn create_and_fund(&self, amount: u64, nonce: u64) -> B256 {
        let offer = self.offer(amount, nonce);
        let digest = self.escrow().hashOffer(offer.clone()).call().await.unwrap();
        let sig = self.seller.sign_hash_sync(&digest).unwrap();
        let receipt = self
            .escrow()
            .createAndFund(offer, sig.as_bytes().into())
            .from(self.buyer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(receipt.status());
        receipt.transaction_hash
    }

    async fn cancel_offer(&self, nonce: u64) {
        let r = self
            .escrow()
            .cancelOffer(U256::from(nonce))
            .from(self.seller.address())
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    async fn mark_delivered(&self, id: u64) {
        let r = self
            .escrow()
            .markDelivered(U256::from(id))
            .from(self.seller.address())
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    async fn dispute(&self, id: u64) {
        let r = self
            .escrow()
            .dispute(U256::from(id))
            .from(self.buyer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    async fn release(&self, id: u64) {
        let r = self
            .escrow()
            .release(U256::from(id))
            .from(self.buyer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    /// 分期报价：每小时一期、共 24 期，买家下单付款。
    async fn stream_create_and_fund(&self, amount: u64, nonce: u64) {
        let offer = StreamEscrow::Offer {
            seller: self.seller.address(),
            buyer: Address::ZERO,
            token: self.usdt,
            amount: U256::from(amount),
            feePayer: 2,
            maxFeeBps: 100,
            maxBaseGas: U256::from(100_000),
            maxClaimGas: U256::from(20_000),
            maxMinFee: U256::from(U),
            bond: U256::ZERO,
            lateSlashBps: 0,
            interval: 0, // Interval.Hour
            periods: 24,
            startWindow: 86_400,
            listingHash: B256::repeat_byte(8),
            expiry: 4_000_000_000,
            maxFills: 1,
            nonce: U256::from(nonce),
        };
        let digest = self.stream().hashOffer(offer.clone()).call().await.unwrap();
        let sig = self.seller.sign_hash_sync(&digest).unwrap();
        let r = self
            .stream()
            .createAndFund(offer, sig.as_bytes().into())
            .from(self.buyer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    async fn stream_start(&self, id: u64) {
        let r = self
            .stream()
            .start(U256::from(id))
            .from(self.seller.address())
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    async fn stream_claim(&self, id: u64) {
        let r = self
            .stream()
            .claim(U256::from(id))
            .from(self.seller.address())
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
    }

    /// 买家发起争议，返回所在区块的时间。
    async fn stream_dispute(&self, id: u64) -> u64 {
        let r = self
            .stream()
            .dispute(U256::from(id))
            .from(self.buyer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(r.status());
        let block = self
            .p
            .get_block_by_hash(r.block_hash.unwrap())
            .await
            .unwrap()
            .unwrap();
        block.header.timestamp
    }

    async fn rpc<
        R: serde::de::DeserializeOwned + std::fmt::Debug + Send + Sync + Unpin + 'static,
    >(
        &self,
        method: &'static str,
        params: Value,
    ) -> R {
        self.p.raw_request(method.into(), params).await.unwrap()
    }

    fn config(&self, anvil: &Anvil, reconcile_ms: u64) -> ChainConfig {
        ChainConfig {
            chain_id: 31_337,
            name: "anvil".into(),
            rpc: RpcConfig {
                primary: Endpoint {
                    http: DEAD_HTTP.into(),
                    ws: Some(DEAD_WS.into()),
                },
                backup: Some(Endpoint {
                    http: anvil.http(),
                    ws: Some(anvil.ws()),
                }),
            },
            contracts: Contracts {
                escrow: self.escrow,
                stream: self.stream,
                config: self.config,
                start_block: self.start_block,
            },
            token: Token {
                symbol: "mUSDT".into(),
                address: self.usdt,
                decimals: 6,
            },
            // anvil：safe = 最新块 − 32，finalized = 最新块 − 64。
            confirmation: ConfirmationPolicy::default(),
            indexer: IndexerPolicy {
                reconcile_interval_ms: reconcile_ms,
                max_block_range: 5,
                promote_every: 1,
                failback_secs: 3600,
            },
            wallet_init_versions: Vec::new(),
            aa: None,
        }
    }
}

struct Running {
    stop: CancellationToken,
    task: tokio::task::JoinHandle<Result<(), forum_indexer::IndexerError>>,
}

impl Running {
    fn start(pool: &PgPool, cfg: ChainConfig) -> Self {
        let stop = CancellationToken::new();
        let task = tokio::spawn(forum_indexer::run(pool.clone(), cfg, None, stop.clone()));
        Self { stop, task }
    }

    async fn stop(self) {
        self.stop.cancel();
        self.task.await.unwrap().unwrap();
    }
}

#[derive(Debug, PartialEq)]
struct OrderRow {
    id: i64,
    state: i16,
    amount: String,
    level: String,
}

/// 一次性担保的订单。
async fn order(pool: &PgPool, chain_order_id: i64) -> Option<OrderRow> {
    let row: Option<(i64, i16, String, String)> = sqlx::query_as(
        "SELECT id, state, amount::text, level::text FROM escrow_orders WHERE chain_order_id = $1 AND mode = 0",
    )
    .bind(chain_order_id)
    .fetch_optional(pool)
    .await
    .unwrap();
    row.map(|(id, state, amount, level)| OrderRow {
        id,
        state,
        amount,
        level,
    })
}

/// 某个一次性担保订单的 order.updated 消息（按写入顺序）。
async fn updates(pool: &PgPool, chain_order_id: i64) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'order.updated' AND (payload->>'chain_order_id')::bigint = $1
           AND payload->>'mode' = 'escrow'
         ORDER BY id",
    )
    .bind(chain_order_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// 订单奖励的免费代付提币次数：(订单行 id, 用户)。
async fn order_credits(pool: &PgPool) -> Vec<(i64, i64)> {
    sqlx::query_as(
        "SELECT order_id, user_id FROM withdraw_credit_ledger WHERE reason = 'order' ORDER BY order_id, user_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// 买卖双方的论坛账号与钱包，以及卖家签出的第 1 号报价卡片（订单 1 由它生成）。返回 (买家, 卖家) 用户 ID。
async fn seed_forum(pool: &PgPool, w: &World) -> (i64, i64) {
    let mut ids = Vec::new();
    for (name, address) in [("buyer", w.buyer), ("seller", w.seller.address())] {
        let id: i64 = sqlx::query_scalar(
            "WITH u AS (INSERT INTO users (email) VALUES ($1 || '@example.com') RETURNING id),
                  k AS (INSERT INTO passkeys (user_id, purpose, credential_id, credential, public_key_cose, pubkey_x, pubkey_y)
                        SELECT id, 1, $1::bytea, '{}', '\\x00', decode(repeat('11', 32), 'hex'), decode(repeat('22', 32), 'hex')
                        FROM u RETURNING id, user_id)
             INSERT INTO wallets (user_id, chain_id, address, passkey_id) SELECT user_id, 31337, $2, id FROM k
             RETURNING user_id",
        )
        .bind(name)
        .bind(address.as_slice())
        .fetch_one(pool)
        .await
        .unwrap();
        ids.push(id);
    }
    let offer_hash = w
        .escrow()
        .hashOffer(w.offer(50 * U, 1))
        .call()
        .await
        .unwrap();
    // 私信里谈好的报价：会话里先有一条消息，付款后订单关联这段谈判记录。
    let (conversation, _) = forum_db::messages::get_or_create(pool, ids[0], ids[1])
        .await
        .unwrap();
    forum_db::messages::insert(
        pool,
        forum_db::messages::NewMessage {
            conversation_id: conversation.id,
            sender_id: Some(ids[0]),
            kind: "text",
            body: "50 USDT 可以吗",
            offer_id: None,
            warn: false,
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO offers (conversation_id, seller_id, buyer_id, chain_id, verifying_contract, seller, buyer, token,
           amount, fee_payer, max_fee_bps, max_base_gas, max_min_fee, bond, late_slash_bps, instant, ship_window,
           confirm_window, listing_hash, expiry, max_fills, nonce, signature, offer_hash, expires_at)
         VALUES ($9, $1, $2, 31337, $3, $4, $5, $6, 50000000, 2, 100, 100000, 1000000, 0, 0, false, 86400, 86400,
           $7, 4000000000, 1, 1, '\\x01', $8, now() + interval '1 day')",
    )
    .bind(ids[1])
    .bind(ids[0])
    .bind(w.escrow.as_slice())
    .bind(w.seller.address().as_slice())
    .bind(Address::ZERO.as_slice())
    .bind(w.usdt.as_slice())
    .bind(B256::repeat_byte(7).as_slice())
    .bind(offer_hash.as_slice())
    .bind(conversation.id)
    .execute(pool)
    .await
    .unwrap();
    (ids[0], ids[1])
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn escrow_lifecycle_reorg_and_finality(pool: PgPool) {
    let Some(bin) = require(anvil_bin(), "anvil", "install Foundry or set ANVIL_BIN") else {
        return;
    };
    let anvil = Anvil::spawn(&bin).unwrap();
    let w = World::deploy(&anvil).await;
    let (buyer_id, seller_id) = seed_forum(&pool, &w).await;

    // ── 阶段 1：校对间隔 1 小时，部署后的订单事件只能经 WebSocket 即时到达。
    // 启动时的校对按 5 块一段补拉：MarketAdded 在第一段就写入，要等游标追上部署后的最新块（这一轮校对结束）
    // 才能探测订阅。否则还没结束的校对可能自己捡到探测事件，订阅（先试不可达的主节点，约 1 秒后才建立）
    // 还没推送就被当成已就绪，之后的订单事件没人补拉（负载下偶发超时）。
    let deployed = i64::try_from(w.p.get_block_number().await.unwrap()).unwrap();
    let indexer = Running::start(&pool, w.config(&anvil, 3_600_000));
    eventually("initial catch-up", || async {
        let cursor: Option<i64> =
            sqlx::query_scalar("SELECT block_number FROM indexer_cursors WHERE chain_id = 31337")
                .fetch_optional(&pool)
                .await
                .unwrap();
        (cursor == Some(deployed)).then_some(())
    })
    .await;
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM escrow_events WHERE name = 'MarketAdded'"
        )
        .await,
        2
    );
    // 订阅建立前的事件只能等校对；先用 OfferCancelled 探测订阅已经在推送。
    for nonce in 1000.. {
        w.cancel_offer(nonce).await;
        let seen = tokio::time::timeout(Duration::from_secs(1), async {
            while count(
                &pool,
                "SELECT count(*) FROM escrow_events WHERE name = 'OfferCancelled'",
            )
            .await
                == 0
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        if seen.is_ok() {
            break;
        }
        assert!(nonce < 1030, "websocket subscription never delivered");
    }
    let tx1 = w.create_and_fund(50 * U, 1).await;
    w.mark_delivered(1).await;
    w.release(1).await;
    let o1 = eventually("order 1 released via websocket", || async {
        order(&pool, 1).await.filter(|o| o.state == 3)
    })
    .await;
    assert_eq!(o1.amount, "50000000");
    assert_eq!(o1.level, "latest");
    let states: Vec<String> = updates(&pool, 1)
        .await
        .iter()
        .map(|u| {
            format!(
                "{}/{}",
                u["state"].as_str().unwrap(),
                u["cause"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(states, ["funded/event", "funded/event", "released/event"]);
    // 信誉与交易经验只认最终确认的订单
    assert_eq!(
        count(&pool, "SELECT count(*) FROM user_trade_stats").await,
        0
    );
    let (fee, gas, confirm, to_seller, snapshot_amount): (String, String, bool, String, String) = sqlx::query_as(
        "SELECT buyer_fee::text, buyer_gas::text, confirm_deadline IS NOT NULL, settled_to_seller::text,
                snapshot->>'amount'
         FROM escrow_orders WHERE chain_order_id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    // 1% 手续费 0.5 USDT 低于最低 1 USDT，按 1 USDT 收，双方各一半；base gas 0.1 USDT 同样平分。
    assert_eq!((fee.as_str(), gas.as_str()), ("500000", "50000"));
    assert!(confirm);
    assert_eq!(snapshot_amount, "50000000");
    assert_eq!(to_seller, (50 * U - 500_000 - 50_000).to_string());
    let tx_in_db: i64 = sqlx::query_scalar("SELECT count(*) FROM escrow_events WHERE tx_hash = $1")
        .bind(tx1.as_slice())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tx_in_db, 1, "OrderCreated stored once");
    // 同一事务里：按钱包地址找到双方用户，生成订单的报价卡片成交次数用完、标为已付款。
    let parties: (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT buyer_id, seller_id FROM escrow_orders WHERE id = $1")
            .bind(o1.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(parties, (Some(buyer_id), Some(seller_id)));
    let status: String = sqlx::query_scalar("SELECT status::text FROM offers")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "paid");
    assert!(
        order_credits(&pool).await.is_empty(),
        "released but not finalized: no withdrawal credit yet"
    );
    // 私信里写入「已付款」卡片，订单关联截至这张卡片的谈判记录；订单聊天按状态变化写入系统消息，哈希链完好。
    let (card, offer_event): (i64, String) =
        sqlx::query_as("SELECT id, body FROM messages WHERE kind = 'offer'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(offer_event, "paid");
    let evidence: i64 =
        sqlx::query_scalar("SELECT last_message_id FROM order_negotiations WHERE order_id = $1")
            .bind(o1.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(evidence, card);
    let chat = forum_db::order_chat::after(&pool, o1.id, 0, 100)
        .await
        .unwrap();
    let bodies: Vec<&str> = chat.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(
        bodies,
        [
            "买家已付款，资金已托管，等待卖家发货",
            "卖家已标记发货，确认期开始",
            "货款已放给卖家，订单完成"
        ]
    );
    assert!(
        chat.iter()
            .all(|m| m.kind == "system" && m.sender_id.is_none())
    );
    assert_eq!(forum_core::chat::verify(&chat).broken_at, None);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM outbox WHERE kind = 'push.order_message'"
        )
        .await,
        3
    );
    indexer.stop().await;

    // ── 阶段 2：重启（从游标继续），校对间隔 200 毫秒。WebSocket 已写入的订单事件不会重复，也不产生修正
    // （订阅建立前漏掉的探测事件此时由校对补上）。
    let order_events = "SELECT count(*) FROM escrow_events WHERE order_id IS NOT NULL";
    let events_before = count(&pool, order_events).await;
    let indexer = Running::start(&pool, w.config(&anvil, 200));
    let head = w.p.get_block_number().await.unwrap();
    eventually("cursor reaches head", || async {
        (count(&pool, "SELECT block_number FROM indexer_cursors").await
            >= i64::try_from(head).unwrap())
        .then_some(())
    })
    .await;
    assert_eq!(count(&pool, order_events).await, events_before);
    let probes = count(
        &pool,
        "SELECT count(*) FROM escrow_events WHERE name = 'OfferCancelled'",
    )
    .await;
    let sent: i64 = w
        .rpc::<Value>(
            "eth_getLogs",
            serde_json::json!([{
                "address": w.escrow, "fromBlock": "0x0", "toBlock": "latest",
                "topics": [Escrow::OfferCancelled::SIGNATURE_HASH],
            }]),
        )
        .await
        .as_array()
        .unwrap()
        .len()
        .try_into()
        .unwrap();
    assert_eq!(
        probes, sent,
        "events missed by the subscription are backfilled"
    );
    assert_eq!(updates(&pool, 1).await.len(), 3);

    // 区块回滚：订单 2 在分叉上下单，随后链回到快照、换一笔金额不同的订单 2 重新打包。
    let snap: U256 = w.rpc("evm_snapshot", Value::Array(vec![])).await;
    let orphan_tx = w.create_and_fund(20 * U, 2).await;
    let o2 = eventually("order 2 on the fork", || async {
        order(&pool, 2).await.filter(|o| o.amount == "20000000")
    })
    .await;
    assert!(w.rpc::<bool>("evm_revert", serde_json::json!([snap])).await);
    let _: Value = w.rpc("anvil_mine", serde_json::json!(["0x2"])).await;
    w.create_and_fund(30 * U, 3).await;
    let o2_new = eventually("order 2 replayed from the main chain", || async {
        order(&pool, 2)
            .await
            .filter(|o| o.amount == "30000000" && o.state == 1)
    })
    .await;
    assert_eq!(o2_new.id, o2.id, "row id stable across the reorg");
    let orphan_left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM escrow_events WHERE tx_hash = $1")
            .bind(orphan_tx.as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(orphan_left, 0, "fork event removed");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM outbox WHERE dedupe_key LIKE 'indexer.deep_reorg%'"
        )
        .await,
        0,
        "a reorg within the safe depth is not an incident"
    );
    let corrections: Vec<Value> = updates(&pool, 2)
        .await
        .into_iter()
        .filter(|u| u["correction"] == true)
        .collect();
    assert!(
        !corrections.is_empty(),
        "correction notification for order 2"
    );
    assert!(corrections.iter().all(|u| u["cause"] == "reorg"));
    // 订单 1 在分叉点之前，不受影响。
    assert!(
        updates(&pool, 1)
            .await
            .iter()
            .all(|u| u["correction"] == false)
    );
    assert_eq!(order(&pool, 1).await.unwrap().state, 3);
    let created: i64 = count(
        &pool,
        "SELECT count(*) FROM escrow_events WHERE name = 'OrderCreated'",
    )
    .await;
    assert_eq!(created, 2);

    // 分期订单：开始后过 2 小时（2 期）领取一次，再由买家发起争议。
    w.stream_create_and_fund(50 * U, 1).await;
    w.stream_start(1).await;
    let _: Value = w.rpc("evm_increaseTime", serde_json::json!([7200])).await;
    let _: Value = w.rpc("evm_mine", Value::Array(vec![])).await;
    w.stream_claim(1).await;
    let stream: (i16, i32, i32, String, String, String) = eventually("stream order claimed", || async {
        sqlx::query_as(
            "SELECT state, interval_secs, periods, released_amount::text, encode(token, 'hex'), snapshot->>'net'
             FROM escrow_orders
             WHERE mode = 1 AND chain_order_id = 1 AND started_at IS NOT NULL AND released_amount > 0",
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
    })
    .await;
    // 50 USDT、各半承担：卖家手续费 0.5、gas 0.05，可解锁净本金 49.45；2/24 期解锁 4.120833（含 0.02 领取 gas）。
    let net = 50 * U - 500_000 - 50_000;
    assert_eq!(
        stream,
        (
            1,
            3600,
            24,
            (net * 2 / 24).to_string(),
            alloy::hex::encode(w.usdt),
            net.to_string()
        )
    );
    let stream_contract: Vec<u8> =
        sqlx::query_scalar("SELECT contract FROM escrow_orders WHERE mode = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stream_contract, w.stream.as_slice());
    // 争议时间取自区块时间（分期解锁冻结于此）。
    let at = w.stream_dispute(1).await;
    let disputed_at: i64 = eventually("stream order disputed", || async {
        sqlx::query_scalar(
            "SELECT extract(epoch FROM disputed_at)::bigint FROM escrow_orders WHERE mode = 1 AND state = 2",
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
    })
    .await;
    assert_eq!(disputed_at, i64::try_from(at).unwrap());
    let stream_id: i64 = sqlx::query_scalar("SELECT id FROM escrow_orders WHERE mode = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let chat = forum_db::order_chat::after(&pool, stream_id, 0, 100)
        .await
        .unwrap();
    let bodies: Vec<&str> = chat.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(
        bodies,
        [
            "买家已付款，资金已托管，等待卖家发货",
            "卖家已开始服务，按周期解锁",
            "买家发起争议，进入举证期"
        ]
    );
    // 回滚：订单 2 的聊天先记下分叉上的付款，再记撤销与主链上的重新付款，链照样完整。
    let chat = forum_db::order_chat::after(&pool, o2.id, 0, 100)
        .await
        .unwrap();
    assert!(chat.len() >= 2, "{chat:?}");
    assert!(chat.iter().any(|m| m.body.contains("回滚")));
    assert_eq!(forum_core::chat::verify(&chat).broken_at, None);

    // ── 阶段 3：再出 100 块，全部事件与订单升级为 finalized，并各发一条确认通知。
    let _: Value = w.rpc("anvil_mine", serde_json::json!(["0x64"])).await;
    eventually("all finalized", || async {
        let pending = count(
            &pool,
            "SELECT count(*) FROM escrow_events WHERE level <> 'finalized'",
        )
        .await;
        let orders = count(
            &pool,
            "SELECT count(*) FROM escrow_orders WHERE level <> 'finalized'",
        )
        .await;
        (pending == 0 && orders == 0).then_some(())
    })
    .await;
    for id in [1, 2] {
        let last = updates(&pool, id).await.pop().unwrap();
        assert_eq!(
            (last["level"].as_str(), last["cause"].as_str()),
            (Some("finalized"), Some("confirmation"))
        );
    }
    // 订单 1（50 USDT 确认放款）最终确认：双方各成功 1 笔、+10 经验；订单 2 未结束不计
    let stats: Vec<(i64, i32, String)> = sqlx::query_as(
        "SELECT user_id, success, volume::text FROM user_trade_stats ORDER BY user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let mut expect = vec![
        (buyer_id, 1, "50000000".to_owned()),
        (seller_id, 1, "50000000".to_owned()),
    ];
    expect.sort_unstable();
    assert_eq!(stats, expect);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM exp_ledger WHERE reason = 4 AND amount = 10"
        )
        .await,
        2
    );
    let (safe, finalized): (i64, i64) =
        sqlx::query_as("SELECT safe_block, finalized_block FROM indexer_cursors")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(safe >= finalized && finalized > 0);
    let before_finality = count(
        &pool,
        "SELECT count(*) FROM indexer_checkpoints WHERE block_number < (SELECT finalized_block FROM indexer_cursors)",
    )
    .await;
    assert!(before_finality <= 1, "checkpoints before finality pruned");
    // 终态（订单 1 已放款）且最终确认：买卖双方各得 1 次免费代付提币；未到终态的订单 2 与分期订单不发。
    let awarded = [(o1.id, buyer_id), (o1.id, seller_id)];
    assert_eq!(order_credits(&pool).await, awarded);
    indexer.stop().await;

    // 同一条链只允许一个 indexer（第二个等待 10 秒后放弃）。
    // 只看本测试库：同一 PostgreSQL 上别的库里跑着的 indexer 也持有同名锁。
    // 先等刚停的 indexer 的锁会话结束，否则下面数到的 1 把锁可能还是它的，第二个就会抢到锁一直跑下去。
    const LOCKS: &str = "SELECT count(*) FROM pg_locks l JOIN pg_database d ON d.oid = l.database \
         WHERE l.locktype = 'advisory' AND l.granted AND d.datname = current_database()";
    eventually("stopped indexer releases the lock", || async {
        (count(&pool, LOCKS).await == 0).then_some(())
    })
    .await;
    let lock_holder = Running::start(&pool, w.config(&anvil, 200));
    eventually("first indexer holds the lock", || async {
        (count(&pool, LOCKS).await == 1).then_some(())
    })
    .await;
    // 第二个 indexer 是另一个进程：用自己的连接池，免得在测试池被第一个占满时卡在取连接上
    let second_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("second pool");
    let second = tokio::time::timeout(
        Duration::from_secs(20),
        forum_indexer::run(
            second_pool,
            w.config(&anvil, 200),
            None,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("second indexer must refuse to start, not run");
    assert!(matches!(
        second,
        Err(forum_indexer::IndexerError::Locked(31_337))
    ));
    lock_holder.stop().await;
    assert_eq!(
        order_credits(&pool).await,
        awarded,
        "restarts and replays award once"
    );
}

#[tokio::test]
async fn rpc_fails_over_to_backup() {
    let Some(bin) = require(anvil_bin(), "anvil", "install Foundry or set ANVIL_BIN") else {
        return;
    };
    let anvil = Anvil::spawn(&bin).unwrap();
    let w = World::deploy(&anvil).await;
    let src = forum_indexer::RpcSource::new(&w.config(&anvil, 1000)).unwrap();
    assert_eq!(src.active(), 0);
    let latest = src.block(BlockTag::Latest).await.unwrap().unwrap();
    assert!(latest.number > 0);
    assert_eq!(src.active(), 1, "switched to the backup endpoint");
    assert_eq!(src.failovers(), 1);
    let logs = src.logs(w.start_block, latest.number).await.unwrap();
    assert!(logs.iter().any(|l| l.contract == w.config));
}

/// 管理员群告警：回滚越过已标为 safe 的区块时发一条（与回滚同一事务）；主节点不通、切到备用节点时发一条
/// （1 小时内不重复）。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn deep_reorg_and_failover_alert_admins(pool: PgPool) {
    let Some(bin) = require(anvil_bin(), "anvil", "install Foundry or set ANVIL_BIN") else {
        return;
    };
    let anvil = Anvil::spawn(&bin).unwrap();
    let w = World::deploy(&anvil).await;
    let indexer = Running::start(&pool, w.config(&anvil, 200));
    let alerts = |prefix: &'static str| {
        let pool = pool.clone();
        async move {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM outbox WHERE kind = 'staff_alert' AND dedupe_key LIKE $1 || '%'",
            )
            .bind(prefix)
            .fetch_one(&pool)
            .await
            .unwrap();
            n
        }
    };

    let snap: U256 = w.rpc("evm_snapshot", Value::Array(vec![])).await;
    w.create_and_fund(20 * U, 1).await;
    let _: Value = w.rpc("anvil_mine", serde_json::json!(["0x40"])).await;
    eventually("order 1 safe", || async {
        order(&pool, 1)
            .await
            .filter(|o| o.level == "safe" || o.level == "finalized")
    })
    .await;
    // 只在 safe 之内的回滚不告警（上面的订单是在新块上确认的，这里还没有回滚）。
    assert_eq!(alerts("indexer.deep_reorg").await, 0);

    assert!(w.rpc::<bool>("evm_revert", serde_json::json!([snap])).await);
    let _: Value = w.rpc("anvil_mine", serde_json::json!(["0x2"])).await;
    eventually("deep reorg alert", || async {
        (alerts("indexer.deep_reorg:31337").await == 1).then_some(())
    })
    .await;
    let text: String = sqlx::query_scalar(
        "SELECT payload->>'text' FROM outbox WHERE dedupe_key = 'indexer.deep_reorg:31337'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(text.contains("越过了已确认为 safe 的区块"), "{text}");

    // 测试链配置的主节点是不通的地址：一启动就切到备用节点，告警只发一次。
    assert_eq!(alerts("indexer.rpc_failover:31337").await, 1);
    indexer.stop().await;
}

/// 仲裁案件跟随链上争议：争议被区块回滚 → 关闭（reverted）；重新打包 → 同一案件重新打开；
/// arbiter 按后端生成的 `resolve` calldata 执行 → 关闭（executed），订单按裁决结算。
#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn arbitration_case_follows_dispute_reorg_and_resolve(pool: PgPool) {
    let Some(bin) = require(anvil_bin(), "anvil", "install Foundry or set ANVIL_BIN") else {
        return;
    };
    let anvil = Anvil::spawn(&bin).unwrap();
    let w = World::deploy(&anvil).await;
    let indexer = Running::start(&pool, w.config(&anvil, 200));
    w.create_and_fund(50 * U, 1).await;
    w.mark_delivered(1).await;
    let order_id = eventually("order funded", || async {
        order(&pool, 1).await.filter(|o| o.state == 1).map(|o| o.id)
    })
    .await;
    let case = |pool: PgPool| async move {
        sqlx::query_as::<_, (i64, Option<String>)>(
            "SELECT id, close_reason FROM arbitration_cases WHERE order_id = $1",
        )
        .bind(order_id)
        .fetch_optional(&pool)
        .await
        .unwrap()
    };

    let snap: U256 = w.rpc("evm_snapshot", Value::Array(vec![])).await;
    w.dispute(1).await;
    let (case_id, _) = eventually("case opened", || async {
        case(pool.clone()).await.filter(|(_, r)| r.is_none())
    })
    .await;
    assert!(w.rpc::<bool>("evm_revert", serde_json::json!([snap])).await);
    let _: Value = w.rpc("anvil_mine", serde_json::json!(["0x3"])).await;
    eventually("case closed by the reorg", || async {
        case(pool.clone())
            .await
            .filter(|(_, r)| r.as_deref() == Some("reverted"))
    })
    .await;
    assert_eq!(order(&pool, 1).await.unwrap().state, 1);

    w.dispute(1).await;
    let (again, _) = eventually("case reopened", || async {
        case(pool.clone()).await.filter(|(_, r)| r.is_none())
    })
    .await;
    assert_eq!(again, case_id, "same case across the reorg");

    let call = forum_chain::arbitration::resolve_call(Market::Escrow, w.escrow, 1, 7_500, 0);
    let tx = alloy::rpc::types::TransactionRequest::default()
        .from(w.arbiter)
        .to(call.to)
        .input(call.data.clone().into());
    let receipt =
        w.p.send_transaction(tx)
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
    assert!(
        receipt.status(),
        "resolve calldata accepted by the contract"
    );
    eventually("case closed by execution", || async {
        case(pool.clone())
            .await
            .filter(|(_, r)| r.as_deref() == Some("executed"))
    })
    .await;
    let bps: Option<i32> =
        sqlx::query_scalar("SELECT ruling_to_buyer_bps FROM escrow_orders WHERE id = $1")
            .bind(order_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bps, Some(7_500));
    indexer.stop().await;
}
