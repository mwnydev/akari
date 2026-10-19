//! 端到端：本机 anvil + PostgreSQL 上真实地代付并代发 UserOperation。
//!
//! 部署 EntryPoint v0.7（`contracts/lib/account-abstraction`，forge 现场编译）、MockUSDT、EscrowConfig、Escrow、
//! FixedFeePaymaster，以及 `fixtures/TestKernelAccount.sol`（ECDSA 所有者、Kernel 同款 `execute` 入口）。
//! 经 forum-sponsor 的 HTTP 接口走完 估算 → 代付签名 → 用户签名 → 提交 → 回执：
//! 1. 首笔操作带 initCode 部署钱包，approve + cancelOffer：收 0.05 USDT，操作生效；
//! 2. 签名后余额被转走、callData 在链上回滚：照样收费，记一次链上失败。
//!
//! 以及：别人的钱包、白名单外的调用、待上链时换 nonce 都被拒绝；余额告警写入发件箱且去重。
//!
//! 需要 `DATABASE_URL`。找不到 anvil 或 forge（`~/.foundry/bin` 或 PATH）时跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolCall;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use forum_chain::aa::{IEntryPoint, SponsorTerms, UserOperation};
use forum_chain::bindings::{Escrow, EscrowConfig, FixedFeePaymaster, MockUSDT};
use forum_chain::config::{AaConfig, ChainConfig, Contracts, Endpoint, RpcConfig, Token};
use forum_db::PgPool;
use forum_sponsor::App;
use forum_sponsor::config::{AlertConfig, Config, Policy, RelayerConfig};
use forum_sponsor::kernel::{self, Call};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const U: u64 = 1_000_000;
const FEE: u64 = 50_000;
const TOKEN: &str = "integration-test-token-0123456789abcdef";
const CHAIN_ID: u64 = 31_337;

alloy::sol! {
    #[sol(rpc)]
    interface ITestKernelFactory {
        function createAccount(address entryPoint, address owner, address token, address paymaster, bytes32 salt)
            external returns (address);
        function getAddress(address entryPoint, address owner, address token, address paymaster, bytes32 salt)
            external view returns (address);
    }
}

fn foundry_bin(name: &str) -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".foundry/bin").join(name))
        .filter(|p| p.is_file())
        .or_else(|| {
            Command::new(name)
                .arg("--version")
                .stdout(Stdio::null())
                .status()
                .ok()
                .filter(|s| s.success())
                .map(|_| PathBuf::from(name))
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
    fn spawn(bin: &Path) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        // 每秒出块：块时间随墙钟前进，代付签名的 validAfter = 当前时间很快就能生效。
        let child = Command::new(bin)
            .args(["--port", &port.to_string(), "--silent", "--block-time", "1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        Self { child, port }
    }

    fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// 在 `target/tmp` 下建一个临时 Foundry 工程编译 fixture（顺带编译 EntryPoint），返回产物目录。
fn compile_fixtures(forge: &Path) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = manifest
        .join("../../../contracts/lib")
        .canonicalize()
        .unwrap();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sponsor-fixtures");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::copy(
        manifest.join("tests/fixtures/TestKernelAccount.sol"),
        root.join("src/TestKernelAccount.sol"),
    )
    .unwrap();
    let lib = lib.display();
    std::fs::write(
        root.join("foundry.toml"),
        format!(
            r#"[profile.default]
src = "src"
out = "out"
libs = ["{lib}"]
allow_paths = ["{lib}"]
solc_version = "0.8.37"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
remappings = ["account-abstraction/={lib}/account-abstraction/contracts/", "@openzeppelin/contracts/={lib}/openzeppelin-contracts/contracts/"]
"#
        ),
    )
    .unwrap();
    let status = Command::new(forge)
        .args(["build", "--quiet", "--root"])
        .arg(&root)
        .status()
        .unwrap();
    assert!(status.success(), "forge build of fixtures failed");
    root.join("out")
}

fn bytecode(out: &Path, file: &str, name: &str) -> Bytes {
    let v: Value = serde_json::from_str(
        &std::fs::read_to_string(out.join(file).join(format!("{name}.json"))).unwrap(),
    )
    .unwrap();
    v["bytecode"]["object"].as_str().unwrap().parse().unwrap()
}

async fn deploy(p: &DynProvider, from: Address, code: Bytes, args: Vec<u8>) -> Address {
    let tx = TransactionRequest::default()
        .from(from)
        .with_deploy_code([code.to_vec(), args].concat());
    let r = p
        .send_transaction(tx)
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(r.status());
    r.contract_address.unwrap()
}

async fn send(p: &DynProvider, from: Address, to: Address, data: Vec<u8>, value: U256) {
    let tx = TransactionRequest::default()
        .from(from)
        .to(to)
        .input(data.into())
        .value(value);
    let r = p
        .send_transaction(tx)
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(r.status());
}

struct World {
    p: DynProvider,
    deployer: Address,
    entry_point: Address,
    usdt: Address,
    escrow: Address,
    paymaster: Address,
    factory: Address,
    pm_signer: PrivateKeySigner,
}

impl World {
    async fn deploy(url: &str, out: &Path) -> Self {
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
        let (deployer, ops) = (accounts[0], accounts[1]);
        let entry_point = deploy(
            &p,
            deployer,
            bytecode(out, "EntryPoint.sol", "EntryPoint"),
            vec![],
        )
        .await;
        let factory = deploy(
            &p,
            deployer,
            bytecode(out, "TestKernelAccount.sol", "TestKernelFactory"),
            vec![],
        )
        .await;
        let usdt = MockUSDT::deploy_builder(&p)
            .from(deployer)
            .deploy()
            .await
            .unwrap();
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
        let pm_signer = PrivateKeySigner::random();
        let paymaster = FixedFeePaymaster::deploy_builder(
            &p,
            entry_point,
            usdt,
            Address::repeat_byte(0x77), // treasury
            pm_signer.address(),
            deployer,
        )
        .from(deployer)
        .deploy()
        .await
        .unwrap();
        send(
            &p,
            deployer,
            paymaster,
            FixedFeePaymaster::depositCall {}.abi_encode(),
            U256::from(10u128.pow(18)),
        )
        .await;
        Self {
            p,
            deployer,
            entry_point,
            usdt,
            escrow,
            paymaster,
            factory,
            pm_signer,
        }
    }

    fn config(&self, rpc: String, db_url: String) -> Config {
        let chain = ChainConfig {
            chain_id: CHAIN_ID,
            name: "anvil".into(),
            rpc: RpcConfig {
                primary: Endpoint {
                    http: rpc,
                    ws: None,
                },
                backup: None,
            },
            contracts: Contracts {
                escrow: self.escrow,
                // 这里不部署 StreamEscrow，占位地址只需与其他合约不同。
                stream: Address::repeat_byte(0x02),
                config: Address::repeat_byte(0x01),
                start_block: 0,
            },
            token: Token {
                symbol: "mUSDT".into(),
                address: self.usdt,
                decimals: 6,
            },
            confirmation: Default::default(),
            indexer: Default::default(),
            wallet_init_versions: Vec::new(),
            aa: None,
        };
        let aa = AaConfig {
            entry_point: self.entry_point,
            paymaster: self.paymaster,
            factories: vec![self.factory],
        };
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            api_token: TOKEN.into(),
            database_url: db_url,
            chain: ChainConfig {
                aa: Some(aa.clone()),
                ..chain
            },
            aa,
            policy: Policy {
                sponsor_fee: u128::from(FEE),
                max_op_cost: U256::from(10u128.pow(17)),
            },
            relayer: RelayerConfig::default(),
            alerts: AlertConfig::default(),
            fallback_bundler: None,
        }
    }

    async fn usdt_balance(&self, who: Address) -> U256 {
        MockUSDT::new(self.usdt, &self.p)
            .balanceOf(who)
            .call()
            .await
            .unwrap()
    }
}

async fn rpc(app: &Router, user: i64, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let req = Request::post("/rpc")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("x-forum-user-id", user.to_string())
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(res.status().is_success(), "{method}: http {}", res.status());
    serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
}

async fn ok(app: &Router, user: i64, method: &str, params: Value) -> Value {
    let r = rpc(app, user, method, params).await;
    assert!(r.get("error").is_none(), "{method}: {r}");
    r["result"].clone()
}

async fn err_code(app: &Router, user: i64, method: &str, params: Value) -> String {
    let r = rpc(app, user, method, params).await;
    r["error"]["data"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("{method}: expected error, got {r}"))
        .to_owned()
}

async fn new_user(db: &PgPool, wallet: Option<Address>) -> i64 {
    let id: i64 = sqlx::query_scalar("INSERT INTO users (email) VALUES ($1) RETURNING id")
        .bind(format!("u{}@example.com", rand_suffix()))
        .fetch_one(db)
        .await
        .unwrap();
    if let Some(w) = wallet {
        let pk: i64 = sqlx::query_scalar(
            "INSERT INTO passkeys (user_id, purpose, credential_id, credential) VALUES ($1, 2, $2, '{}') RETURNING id",
        )
        .bind(id)
        .bind(rand_suffix().to_be_bytes().to_vec())
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO wallets (user_id, chain_id, address, passkey_id) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(CHAIN_ID as i64)
        .bind(w.to_vec())
        .bind(pk)
        .execute(db)
        .await
        .unwrap();
    }
    id
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// 钱包前端的流程：占位代付 → 估算 → gas 价格 → 正式代付 → 用户签名。
async fn prepare(
    app: &Router,
    user: i64,
    w: &World,
    mut op: UserOperation,
    owner: &PrivateKeySigner,
) -> UserOperation {
    let ep = json!(w.entry_point);
    let chain = json!(format!("{CHAIN_ID:#x}"));
    let stub = ok(
        app,
        user,
        "pm_getPaymasterStubData",
        json!([op, ep, chain, {}]),
    )
    .await;
    assert_eq!(stub["isFinal"], false);
    apply_paymaster(&mut op, &stub);
    let gas = ok(app, user, "eth_estimateUserOperationGas", json!([op, ep])).await;
    op.call_gas_limit = gas["callGasLimit"].as_str().unwrap().parse().unwrap();
    op.verification_gas_limit = gas["verificationGasLimit"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    op.pre_verification_gas = gas["preVerificationGas"].as_str().unwrap().parse().unwrap();
    let price = ok(app, user, "pimlico_getUserOperationGasPrice", json!([])).await;
    op.max_fee_per_gas = price["fast"]["maxFeePerGas"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    op.max_priority_fee_per_gas = price["fast"]["maxPriorityFeePerGas"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let data = ok(app, user, "pm_getPaymasterData", json!([op, ep, chain, {}])).await;
    apply_paymaster(&mut op, &data);
    sign(&mut op, w, owner).await;
    op
}

fn apply_paymaster(op: &mut UserOperation, f: &Value) {
    op.paymaster = Some(f["paymaster"].as_str().unwrap().parse().unwrap());
    op.paymaster_data = Some(f["paymasterData"].as_str().unwrap().parse().unwrap());
    op.paymaster_verification_gas_limit = Some(
        f["paymasterVerificationGasLimit"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    op.paymaster_post_op_gas_limit = Some(
        f["paymasterPostOpGasLimit"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
}

async fn sign(op: &mut UserOperation, w: &World, owner: &PrivateKeySigner) {
    let hash = op.hash(w.entry_point, CHAIN_ID).unwrap();
    // 本地复刻的两个哈希必须与链上合约一致。
    let ep = IEntryPoint::new(w.entry_point, &w.p);
    assert_eq!(
        ep.getUserOpHash(op.pack().unwrap()).call().await.unwrap(),
        hash
    );
    let terms = SponsorTerms::decode(op.paymaster_data.as_ref().unwrap()).unwrap();
    let pm = FixedFeePaymaster::new(w.paymaster, &w.p);
    let packed = op.pack().unwrap();
    let onchain = pm
        .getHash(
            FixedFeePaymaster::PackedUserOperation {
                sender: packed.sender,
                nonce: packed.nonce,
                initCode: packed.initCode,
                callData: packed.callData,
                accountGasLimits: packed.accountGasLimits,
                preVerificationGas: packed.preVerificationGas,
                gasFees: packed.gasFees,
                paymasterAndData: packed.paymasterAndData,
                signature: packed.signature,
            },
            terms.valid_until.try_into().unwrap(),
            terms.valid_after.try_into().unwrap(),
            terms.fee,
        )
        .call()
        .await
        .unwrap();
    assert_eq!(onchain, terms.hash(op, CHAIN_ID, w.paymaster).unwrap());
    op.signature = owner
        .sign_message_sync(hash.as_slice())
        .unwrap()
        .as_bytes()
        .to_vec()
        .into();
}

async fn receipt(app: &Router, user: i64, hash: &Value) -> Value {
    for _ in 0..150 {
        let r = ok(app, user, "eth_getUserOperationReceipt", json!([hash])).await;
        if !r.is_null() {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("no receipt for {hash}");
}

fn hash_of(v: &Value) -> B256 {
    serde_json::from_value(v.clone()).unwrap()
}

async fn status(db: &PgPool, hash: &Value) -> String {
    let h = hash_of(hash);
    sqlx::query_scalar("SELECT status::text FROM sponsorships WHERE user_op_hash = $1")
        .bind(h.to_vec())
        .fetch_one(db)
        .await
        .unwrap()
}

fn usdt_call(to: Address, data: impl SolCall) -> Call {
    Call {
        target: to,
        value: U256::ZERO,
        data: data.abi_encode().into(),
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn sponsors_and_relays_user_operations(db: PgPool) {
    let (Some(anvil_bin), Some(forge)) = (foundry_bin("anvil"), foundry_bin("forge")) else {
        eprintln!("anvil or forge not found, skipping");
        return;
    };
    forum_runtime::init_tracing();
    let out = compile_fixtures(&forge);
    let anvil = Anvil::spawn(&anvil_bin);
    let w = World::deploy(&anvil.http(), &out).await;

    // 代发热钱包：anvil 第 9 号账户（公开助记词），有 10000 ETH。
    let hot: PrivateKeySigner =
        "0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6"
            .parse()
            .unwrap();
    let opts = db.connect_options();
    let db_url = format!(
        "postgres://{}@{}:{}/{}",
        opts.get_username(),
        opts.get_host(),
        opts.get_port(),
        opts.get_database().unwrap()
    );
    let app = App::new(
        w.config(anvil.http(), db_url),
        db.clone(),
        w.pm_signer.clone(),
        hot,
    )
    .unwrap();
    app.verify_onchain().await.unwrap();
    let shutdown = CancellationToken::new();
    let tasks = app.spawn_background(&shutdown);
    let router = app.router();

    // 用户 A 的钱包：尚未部署的 counterfactual 地址，先往里打 10 USDT。所有者用 anvil 第 2 号账户
    // （公开助记词）：既能签 UserOperation，也能直接发交易调用自己的账户。
    let owner: PrivateKeySigner =
        "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
            .parse()
            .unwrap();
    let salt = B256::ZERO;
    let factory_data = ITestKernelFactory::createAccountCall {
        entryPoint: w.entry_point,
        owner: owner.address(),
        token: w.usdt,
        paymaster: w.paymaster,
        salt,
    }
    .abi_encode();
    let sender = ITestKernelFactory::new(w.factory, &w.p)
        .getAddress(w.entry_point, owner.address(), w.usdt, w.paymaster, salt)
        .call()
        .await
        .unwrap();
    send(
        &w.p,
        w.deployer,
        w.usdt,
        MockUSDT::mintCall {
            to: sender,
            amount: U256::from(10 * U),
        }
        .abi_encode(),
        U256::ZERO,
    )
    .await;
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

    // 2. 提回交易所 9 USDT：模拟通过并签名后，用户（所有者直接调用账户）先把钱转走，链上 callData 回滚，
    //    代付费照样在验证阶段扣走。
    let exchange = Address::repeat_byte(0xe1);
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
    let transfer = prepare(&router, user_a, &w, transfer, &owner).await;
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

    // 3. 过期对账：被取代的旧签名没上链 → expired；把已上链的第一笔改回「已提交、已过期」，
    //    对账从日志里找回它的结果。
    let first_hash = hash_of(&old_hash);
    sqlx::query("UPDATE sponsorships SET status = 'submitted', valid_until = now() - interval '1 minute' WHERE user_op_hash = $1")
        .bind(included_hash.to_vec())
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sponsorships SET valid_until = now() - interval '1 minute' WHERE user_op_hash = $1",
    )
    .bind(first_hash.to_vec())
    .execute(&db)
    .await
    .unwrap();
    let n = forum_sponsor::ledger::reconcile_once(&db, &app.chain, 2000)
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(status(&db, &old_hash).await, "expired");
    assert_eq!(status(&db, &json!(included_hash)).await, "included");

    // 3. 余额告警：告警线调高后写入发件箱，再查一次不重复写。
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
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM outbox WHERE kind LIKE 'alert.%' ORDER BY kind")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        kinds,
        ["alert.hot_wallet_low", "alert.paymaster_deposit_low"]
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
