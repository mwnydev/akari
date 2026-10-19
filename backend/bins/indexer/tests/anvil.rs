//! 端到端：本机 anvil + PostgreSQL。
//!
//! 部署 MockUSDT、EscrowConfig、Escrow（用 `crates/chain/abi` 里的字节码），走一遍 createAndFund → markDelivered
//! → release，检查订单表、事件表与发件箱；再用 `evm_snapshot` / `evm_revert` 制造区块回滚，检查撤销与重放；
//! 最后多出块，检查升级到 finalized。主节点故意配置成不可达，同时验证主备切换。
//!
//! 需要 `DATABASE_URL`（sqlx::test 为每个用例建临时库）。找不到 anvil（`ANVIL_BIN`、`~/.foundry/bin/anvil`
//! 或 PATH）时跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::future::Future;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use alloy::primitives::{Address, B256, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolEvent;
use forum_chain::bindings::{Escrow, EscrowConfig, MockUSDT};
use forum_chain::config::{
    ChainConfig, ConfirmationPolicy, Contracts, Endpoint, IndexerPolicy, RpcConfig, Token,
};
use forum_chain::reorg::{BlockTag, ChainSource};
use forum_db::PgPool;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// anvil 默认助记词的第 1 号账户私钥（卖家，签报价用）。
const SELLER_KEY: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const U: u64 = 1_000_000;
/// 不可达的主节点：验证主备切换。
const DEAD_HTTP: &str = "http://127.0.0.1:1";
const DEAD_WS: &str = "ws://127.0.0.1:1";

fn anvil_bin() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("ANVIL_BIN").map(PathBuf::from),
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".foundry/bin/anvil")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
        .or_else(|| {
            Command::new("anvil")
                .arg("--version")
                .stdout(Stdio::null())
                .status()
                .ok()
                .filter(|s| s.success())
                .map(|_| PathBuf::from("anvil"))
        })
}

struct Anvil {
    child: Child,
    port: u16,
}

impl Drop for Anvil {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Anvil {
    fn spawn(bin: &PathBuf) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(bin)
            .args(["--port", &port.to_string(), "--silent"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        Self { child, port }
    }

    fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn ws(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }
}

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
    buyer: Address,
    seller: PrivateKeySigner,
    start_block: u64,
}

impl World {
    async fn deploy(url: &str) -> Self {
        // 不在客户端填 nonce / gas：anvil 的 eth_sendTransaction 会自己填，`evm_revert` 之后也不会用到过期的 nonce。
        let p = ProviderBuilder::new()
            .disable_recommended_fillers()
            .connect_http(url.parse().unwrap())
            .erased();
        for _ in 0..100 {
            if p.get_block_number().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
        // 与 Deploy.s.sol 相同：先算出 Escrow 的地址登记为市场，再部署 Config、Escrow。
        let nonce = p.get_transaction_count(deployer).await.unwrap();
        let escrow_addr = deployer.create(nonce + 1);
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
        let config =
            EscrowConfig::deploy_builder(&p, ops, params, usdt, Address::ZERO, vec![escrow_addr])
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
        token
            .approve(escrow, U256::MAX)
            .from(buyer)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        Self {
            p,
            usdt,
            config,
            escrow,
            buyer,
            seller,
            start_block,
        }
    }

    fn escrow(&self) -> Escrow::EscrowInstance<&DynProvider> {
        Escrow::new(self.escrow, &self.p)
    }

    /// 买家按卖家签名的报价下单付款，返回交易哈希。
    async fn create_and_fund(&self, amount: u64, nonce: u64) -> B256 {
        let offer = Escrow::Offer {
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
        };
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
        let task = tokio::spawn(forum_indexer::run(pool.clone(), cfg, stop.clone()));
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

async fn order(pool: &PgPool, chain_order_id: i64) -> Option<OrderRow> {
    let row: Option<(i64, i16, String, String)> = sqlx::query_as(
        "SELECT id, state, amount::text, level::text FROM escrow_orders WHERE chain_order_id = $1",
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

/// 某个链上订单的 order.updated 消息（按写入顺序）。
async fn updates(pool: &PgPool, chain_order_id: i64) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE kind = 'order.updated' AND (payload->>'chain_order_id')::bigint = $1
         ORDER BY id",
    )
    .bind(chain_order_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn escrow_lifecycle_reorg_and_finality(pool: PgPool) {
    let Some(bin) = anvil_bin() else {
        eprintln!("skipped: anvil not found (install Foundry or set ANVIL_BIN)");
        return;
    };
    let anvil = Anvil::spawn(&bin);
    let w = World::deploy(&anvil.http()).await;

    // ── 阶段 1：校对间隔 1 小时，部署后的订单事件只能经 WebSocket 即时到达。
    let indexer = Running::start(&pool, w.config(&anvil, 3_600_000));
    eventually("initial catch-up", || async {
        (count(
            &pool,
            "SELECT count(*) FROM escrow_events WHERE name = 'MarketAdded'",
        )
        .await
            == 1)
            .then_some(())
    })
    .await;
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
    indexer.stop().await;

    // 同一条链只允许一个 indexer（第二个等待 10 秒后放弃）。
    let lock_holder = Running::start(&pool, w.config(&anvil, 200));
    eventually("first indexer holds the lock", || async {
        (count(
            &pool,
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND granted",
        )
        .await
            == 1)
            .then_some(())
    })
    .await;
    let second = tokio::time::timeout(
        Duration::from_secs(20),
        forum_indexer::run(
            pool.clone(),
            w.config(&anvil, 200),
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
}

#[tokio::test]
async fn rpc_fails_over_to_backup() {
    let Some(bin) = anvil_bin() else {
        eprintln!("skipped: anvil not found (install Foundry or set ANVIL_BIN)");
        return;
    };
    let anvil = Anvil::spawn(&bin);
    let w = World::deploy(&anvil.http()).await;
    let src = forum_indexer::RpcSource::new(&w.config(&anvil, 1000)).unwrap();
    assert_eq!(src.active(), 0);
    let latest = src.block(BlockTag::Latest).await.unwrap().unwrap();
    assert!(latest.number > 0);
    assert_eq!(src.active(), 1, "switched to the backup endpoint");
    let logs = src.logs(w.start_block, latest.number).await.unwrap();
    assert!(logs.iter().any(|l| l.contract == w.config));
}
