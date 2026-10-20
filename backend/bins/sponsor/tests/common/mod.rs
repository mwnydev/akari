//! forum-sponsor 端到端测试共用：本机 anvil、编译 fixture（EntryPoint 与测试账户）、部署合约、启动 forum-sponsor，
//! 以及钱包前端的代付流程（占位 → 估算 → 正式代付 → 用户签名）。forum-api 的拍卖端到端测试也经 `#[path]` 引用本文件，
//! 所以 fixture 与合约库的路径都按 `bins/<crate>` 的相对位置写。

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // 测试代码；各测试文件只用其中一部分

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
use forum_sponsor::kernel::Call;
use serde_json::{Value, json};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

pub const U: u64 = 1_000_000;
pub const FEE: u64 = 50_000;
pub const TOKEN: &str = "integration-test-token-0123456789abcdef";
pub const CHAIN_ID: u64 = 31_337;

alloy::sol! {
    #[sol(rpc)]
    interface ITestKernelFactory {
        function createAccount(address entryPoint, address owner, address token, address paymaster, bytes32 salt)
            external returns (address);
        function getAddress(address entryPoint, address owner, address token, address paymaster, bytes32 salt)
            external view returns (address);
    }
}

pub fn foundry_bin(name: &str) -> Option<PathBuf> {
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

pub struct Anvil {
    pub child: Child,
    pub port: u16,
}

impl Drop for Anvil {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Anvil {
    pub fn spawn(bin: &Path) -> Self {
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

    pub fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn ws(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }
}

/// 在 `target/tmp` 下建一个临时 Foundry 工程编译 fixture（顺带编译 EntryPoint），返回产物目录。
/// 同一进程里的用例共用一次编译（并发编译同一目录会互相覆盖）。
pub fn compile_fixtures(forge: &Path) -> PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| compile(forge)).clone()
}

pub fn compile(forge: &Path) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = manifest
        .join("../../../contracts/lib")
        .canonicalize()
        .unwrap();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sponsor-fixtures");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::copy(
        manifest.join("../sponsor/tests/fixtures/TestKernelAccount.sol"),
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

pub fn bytecode(out: &Path, file: &str, name: &str) -> Bytes {
    let v: Value = serde_json::from_str(
        &std::fs::read_to_string(out.join(file).join(format!("{name}.json"))).unwrap(),
    )
    .unwrap();
    v["bytecode"]["object"].as_str().unwrap().parse().unwrap()
}

pub async fn deploy(p: &DynProvider, from: Address, code: Bytes, args: Vec<u8>) -> Address {
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

pub async fn send(p: &DynProvider, from: Address, to: Address, data: Vec<u8>, value: U256) {
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

pub struct World {
    pub p: DynProvider,
    pub deployer: Address,
    pub entry_point: Address,
    pub usdt: Address,
    pub escrow: Address,
    pub paymaster: Address,
    pub factory: Address,
    pub pm_signer: PrivateKeySigner,
    pub config: Address,
    /// `EscrowConfig.auctionSigner`：forum-sponsor 用它签拍卖成交证明。
    pub auction_signer: PrivateKeySigner,
}

impl World {
    pub async fn deploy(url: &str, out: &Path) -> Self {
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
        let auction_signer = PrivateKeySigner::random();
        let config = EscrowConfig::deploy_builder(
            &p,
            ops,
            params,
            usdt,
            auction_signer.address(),
            vec![escrow_addr],
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
            config,
            auction_signer,
        }
    }

    pub fn config(&self, rpc: String, db_url: String) -> Config {
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
                config: self.config,
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

    pub async fn usdt_balance(&self, who: Address) -> U256 {
        MockUSDT::new(self.usdt, &self.p)
            .balanceOf(who)
            .call()
            .await
            .unwrap()
    }
}

pub async fn rpc(app: &Router, user: i64, method: &str, params: Value) -> Value {
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

pub async fn ok(app: &Router, user: i64, method: &str, params: Value) -> Value {
    let r = rpc(app, user, method, params).await;
    assert!(r.get("error").is_none(), "{method}: {r}");
    r["result"].clone()
}

pub async fn err_code(app: &Router, user: i64, method: &str, params: Value) -> String {
    let r = rpc(app, user, method, params).await;
    r["error"]["data"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("{method}: expected error, got {r}"))
        .to_owned()
}

pub async fn new_user(db: &PgPool, wallet: Option<Address>) -> i64 {
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

pub fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// 钱包前端的流程：占位代付 → 估算 → gas 价格 → 正式代付 → 用户签名。
pub async fn prepare(
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

pub fn apply_paymaster(op: &mut UserOperation, f: &Value) {
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

pub async fn sign(op: &mut UserOperation, w: &World, owner: &PrivateKeySigner) {
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

pub async fn receipt(app: &Router, user: i64, hash: &Value) -> Value {
    for _ in 0..150 {
        let r = ok(app, user, "eth_getUserOperationReceipt", json!([hash])).await;
        if !r.is_null() {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("no receipt for {hash}");
}

pub fn hash_of(v: &Value) -> B256 {
    serde_json::from_value(v.clone()).unwrap()
}

pub async fn status(db: &PgPool, hash: &Value) -> String {
    let h = hash_of(hash);
    sqlx::query_scalar("SELECT status::text FROM sponsorships WHERE user_op_hash = $1")
        .bind(h.to_vec())
        .fetch_one(db)
        .await
        .unwrap()
}

/// 免费代付提币次数余额。
pub async fn credits(db: &PgPool, user: i64) -> i64 {
    let since = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    forum_db::credits::usage(db, user, since, None)
        .await
        .unwrap()
        .balance
}

pub fn usdt_call(to: Address, data: impl SolCall) -> Call {
    Call {
        target: to,
        value: U256::ZERO,
        data: data.abi_encode().into(),
    }
}

/// 连接 `rpc` 的 forum-sponsor（启动检查已通过）。代发热钱包是 anvil 第 9 号账户（公开助记词），有 10000 ETH。
pub async fn launch(db: &PgPool, w: &World, rpc: String) -> App {
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
        w.config(rpc, db_url),
        db.clone(),
        w.pm_signer.clone(),
        hot,
        Some(w.auction_signer.clone()),
    )
    .unwrap();
    app.verify_onchain().await.unwrap();
    app
}

/// 尚未部署的钱包（counterfactual 地址），先往里打 `usdt` 最小单位。所有者用 anvil 第 2 号账户（公开助记词）：
/// 既能签 UserOperation，也能直接发交易调用自己的账户。返回所有者、钱包地址与部署用的 factoryData。
pub async fn wallet(w: &World, salt: B256, usdt: u64) -> (PrivateKeySigner, Address, Vec<u8>) {
    let owner: PrivateKeySigner =
        "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
            .parse()
            .unwrap();
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
            amount: U256::from(usdt),
        }
        .abi_encode(),
        U256::ZERO,
    )
    .await;
    (owner, sender, factory_data)
}

/// Solady 的 ERC-6492 回滚式验证器（`0x00007bd7…`，Arbitrum 上已有，`Escrow` 验未部署钱包的签名要用），
/// 经标准 CREATE2 部署器装到空白 anvil 上（与 `contracts/test/EscrowSmartAccount.t.sol` 相同的盐与创建码）。
pub async fn deploy_erc6492_verifier(w: &World) {
    let deployer: Address = "0x4e59b44847b379578588920cA78FbF26c0B4956C"
        .parse()
        .unwrap();
    let salt: B256 = "0x000000000000000000000000000000000000000068f35e1510740001fd13984a"
        .parse()
        .unwrap();
    let initcode: Bytes = "0x6040600b3d3960403df3fe36383d373d3d6020515160208051013d3d515af160203851516084018038385101606037303452813582523838523490601c34355afa34513060e01b141634fd"
        .parse()
        .unwrap();
    send(
        &w.p,
        w.deployer,
        deployer,
        [salt.as_slice(), initcode.as_ref()].concat(),
        U256::ZERO,
    )
    .await;
    let verifier: Address = "0x00007bd799e4A591FeA53f8A8a3E9f931626Ba7e"
        .parse()
        .unwrap();
    assert!(!w.p.get_code_at(verifier).await.unwrap().is_empty());
}

/// ERC-6492 包装：未部署账户的签名 = `abi.encode(工厂, 工厂调用, 内层签名) ‖ 魔数`。
pub fn erc6492_wrap(factory: Address, factory_data: &[u8], inner: &[u8]) -> Vec<u8> {
    use alloy::sol_types::SolValue;
    let magic = [0x64u8, 0x92].repeat(16);
    let body = (
        factory,
        Bytes::copy_from_slice(factory_data),
        Bytes::copy_from_slice(inner),
    )
        .abi_encode_params();
    [body, magic].concat()
}
