//! 提回交易所的代付规则（plan 第 5 节「提币地址冷却」「gas 规则」）：USDT `transfer` 只代付转到本人已生效的提币地址，
//! 冷却中、已撤销的地址与别人的地址都拒绝；通过的照样收代付费。另需 1 次免费代付提币次数，每人滚动 24 小时最多 3 次。
//!
//! 只走 `pm_getPaymasterStubData` 的规则检查，不连链（RPC 指向一个不监听的端口）。需要 `DATABASE_URL`。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, U64, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolCall;
use forum_chain::aa::{SponsorTerms, UserOperation};
use forum_chain::config::{AaConfig, Endpoint};
use forum_chain::rpc::FailoverRpc;
use forum_core::ErrorCode;
use forum_db::PgPool;
use forum_sponsor::chain::Chain;
use forum_sponsor::config::Policy;
use forum_sponsor::jsonrpc::{Error, PAYMASTER_REJECTED, RpcError};
use forum_sponsor::kernel::{self, Call};
use forum_sponsor::policy::Contracts;
use forum_sponsor::sponsor::Sponsor;

const CHAIN_ID: u64 = 421_614;
const FEE: u128 = 50_000;
const TOKEN: Address = Address::repeat_byte(0x01);
const ENTRY_POINT: Address = Address::repeat_byte(0xe0);
const WALLET: Address = Address::repeat_byte(0xaa);

alloy::sol! {
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
    }
}

fn sponsor(db: PgPool) -> Sponsor {
    let unused = Endpoint {
        http: "http://127.0.0.1:9".into(),
        ws: None,
    };
    let chain = Chain {
        rpc: FailoverRpc::new(&[&unused], Duration::from_secs(300)).unwrap(),
        chain_id: CHAIN_ID,
        entry_point: ENTRY_POINT,
    };
    Sponsor::new(
        db,
        Arc::new(chain),
        Contracts {
            token: TOKEN,
            escrow: Address::repeat_byte(0x02),
            stream: Address::repeat_byte(0x03),
        },
        AaConfig {
            entry_point: ENTRY_POINT,
            paymaster: Address::repeat_byte(0x04),
            factories: vec![Address::repeat_byte(0x05)],
        },
        Policy {
            sponsor_fee: FEE,
            max_op_cost: U256::from(10u128.pow(17)),
        },
        PrivateKeySigner::random(),
        Address::repeat_byte(0x06),
    )
}

async fn user_with_wallet(db: &PgPool) -> i64 {
    let id: i64 =
        sqlx::query_scalar("INSERT INTO users (email) VALUES ('w@example.com') RETURNING id")
            .fetch_one(db)
            .await
            .unwrap();
    let pk: i64 = sqlx::query_scalar(
        "INSERT INTO passkeys (user_id, purpose, credential_id, credential) VALUES ($1, 2, '\\x01', '{}') RETURNING id",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO wallets (user_id, chain_id, address, passkey_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(CHAIN_ID as i64)
    .bind(WALLET.to_vec())
    .bind(pk)
    .execute(db)
    .await
    .unwrap();
    id
}

/// 登记一个提币地址：`active_in` 秒后生效（负数 = 已生效），`revoked` 为已撤销。
async fn add_address(db: &PgPool, user_id: i64, to: Address, active_in: i64, revoked: bool) {
    sqlx::query(
        "INSERT INTO withdraw_addresses (user_id, chain_id, address, label, active_at, revoked_at)
         VALUES ($1, $2, $3, 'ex', now() + make_interval(secs => $4), CASE WHEN $5 THEN now() END)",
    )
    .bind(user_id)
    .bind(CHAIN_ID as i64)
    .bind(to.to_vec())
    .bind(active_in as f64)
    .bind(revoked)
    .execute(db)
    .await
    .unwrap();
}

fn transfer(to: Address, nonce: u64) -> UserOperation {
    let call = Call {
        target: TOKEN,
        value: U256::ZERO,
        data: IERC20::transferCall {
            to,
            amount: U256::from(1_000_000),
        }
        .abi_encode()
        .into(),
    };
    UserOperation {
        sender: WALLET,
        nonce: U256::from(nonce),
        call_data: kernel::encode(&[call]),
        ..UserOperation::default()
    }
}

async fn stub_nonce(s: &Sponsor, user_id: i64, to: Address, nonce: u64) -> Result<u128, ErrorCode> {
    match s
        .stub(
            user_id,
            &transfer(to, nonce),
            ENTRY_POINT,
            U64::from(CHAIN_ID),
        )
        .await
    {
        Ok(f) => Ok(SponsorTerms::decode(&f.paymaster_data).unwrap().fee),
        Err(Error::Reject(r)) => Err(r.code),
        Err(Error::Internal(e)) => panic!("internal error: {e}"),
    }
}

async fn stub(s: &Sponsor, user_id: i64, to: Address) -> Result<u128, ErrorCode> {
    stub_nonce(s, user_id, to, 0).await
}

/// 一笔已经签过的代付提币（同 forum-sponsor 签名时写入的记录与扣减），`age_secs` 秒前签的。
async fn withdrawn(db: &PgPool, user_id: i64, nonce: u64, status: &str, age_secs: i64) {
    let hash = alloy::primitives::keccak256(format!("{user_id}:{nonce}:{status}:{age_secs}"));
    sqlx::query(
        "WITH s AS (
           INSERT INTO sponsorships (user_id, chain_id, sender, nonce, user_op_hash, fee, valid_until, signed_block,
                                     status, created_at)
           VALUES ($1, $2, $3, $4, $5, 50000, now() + interval '2 minutes', 1, $6::sponsorship_status,
                   now() - make_interval(secs => $7))
           RETURNING id, created_at
         )
         INSERT INTO withdraw_credit_ledger (user_id, delta, reason, sponsorship_id, created_at)
         SELECT $1, -1, 'withdraw', id, created_at FROM s",
    )
    .bind(user_id)
    .bind(CHAIN_ID as i64)
    .bind(WALLET.to_vec())
    .bind(nonce as i64)
    .bind(hash.to_vec())
    .bind(status)
    .bind(age_secs as f64)
    .execute(db)
    .await
    .unwrap();
}

/// 给用户完成 `n` 笔已放款且最终确认的担保订单，每笔照 forum-indexer 的做法发次数。
async fn finish_orders(db: &PgPool, user_id: i64, n: i64) {
    sqlx::query(
        "INSERT INTO chains (chain_id, name, escrow_address, config_address, token_address, token_symbol,
                             token_decimals, start_block, safe_depth, finalized_depth)
         VALUES ($1, 'test', '\\x0202020202020202020202020202020202020202', '\\x0707070707070707070707070707070707070707',
                 '\\x0101010101010101010101010101010101010101', 'USDT', 6, 0, 1, 1)
         ON CONFLICT DO NOTHING",
    )
    .bind(CHAIN_ID as i64)
    .execute(db)
    .await
    .unwrap();
    let ids: Vec<i64> = sqlx::query_scalar(
        "INSERT INTO escrow_orders (chain_id, contract, chain_order_id, buyer, seller, buyer_id, token, amount,
                                    instant, offer_hash, listing_hash, state, level, created_block, updated_block)
         SELECT $1, '\\x0202020202020202020202020202020202020202', coalesce(
                  (SELECT max(chain_order_id) FROM escrow_orders), 0) + g, $3, '\\x0909090909090909090909090909090909090909',
                $2, '\\x0101010101010101010101010101010101010101', 5000000, false, '\\x01', '\\x02', 3, 'finalized', 1, 1
         FROM generate_series(1, $4) g
         RETURNING id",
    )
    .bind(CHAIN_ID as i64)
    .bind(user_id)
    .bind(WALLET.to_vec())
    .bind(n)
    .fetch_all(db)
    .await
    .unwrap();
    for id in ids {
        assert_eq!(forum_db::credits::award_order(db, id).await.unwrap(), 1);
    }
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn transfers_only_to_active_withdraw_addresses(db: PgPool) {
    let s = sponsor(db.clone());
    let user = user_with_wallet(&db).await;
    let active = Address::repeat_byte(0x10);
    let pending = Address::repeat_byte(0x11);
    let revoked = Address::repeat_byte(0x12);
    let unknown = Address::repeat_byte(0x13);
    add_address(&db, user, active, -60, false).await;
    add_address(&db, user, pending, 86_400, false).await;
    add_address(&db, user, revoked, -60, true).await;
    forum_db::credits::award_starter(&db, user).await.unwrap();

    // 已生效：收代付费（付款前档位）。
    assert_eq!(stub(&s, user, active).await, Ok(FEE));
    for to in [pending, revoked, unknown, WALLET] {
        assert_eq!(
            stub(&s, user, to).await,
            Err(ErrorCode::WithdrawAddressNotActive),
            "{to}"
        );
    }

    // 别人已生效的地址也不行：只认本人的。
    let other: i64 =
        sqlx::query_scalar("INSERT INTO users (email) VALUES ('o@example.com') RETURNING id")
            .fetch_one(&db)
            .await
            .unwrap();
    let theirs = Address::repeat_byte(0x14);
    add_address(&db, other, theirs, -60, false).await;
    assert_eq!(
        stub(&s, user, theirs).await,
        Err(ErrorCode::WithdrawAddressNotActive)
    );

    // 冷却期满即可代付。
    sqlx::query("UPDATE withdraw_addresses SET active_at = now() WHERE address = $1")
        .bind(pending.to_vec())
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(stub(&s, user, pending).await, Ok(FEE));
}

#[sqlx::test(migrator = "forum_db::MIGRATOR")]
async fn withdrawals_consume_credits_under_a_daily_cap(db: PgPool) {
    let s = sponsor(db.clone());
    let user = user_with_wallet(&db).await;
    let exchange = Address::repeat_byte(0x10);
    add_address(&db, user, exchange, -60, false).await;

    // 没有次数：拒绝，JSON-RPC 错误码是 Paymaster 拒绝，提示里有自付 ETH 的出路。
    assert_eq!(
        stub(&s, user, exchange).await,
        Err(ErrorCode::WithdrawCreditRequired)
    );
    let Err(err) = s
        .stub(
            user,
            &transfer(exchange, 0),
            ENTRY_POINT,
            U64::from(CHAIN_ID),
        )
        .await
    else {
        panic!("expected a rejection");
    };
    let rpc = RpcError::from(err);
    assert_eq!(rpc.code, PAYMASTER_REJECTED);
    assert_eq!(rpc.code, -32501);
    assert!(rpc.message.contains("紧急取回"), "{}", rpc.message);

    // 开通奖励只有一次。
    assert!(forum_db::credits::award_starter(&db, user).await.unwrap());
    assert!(!forum_db::credits::award_starter(&db, user).await.unwrap());
    assert_eq!(stub(&s, user, exchange).await, Ok(FEE));

    // 用掉之后（已上链）没有次数了；同一 nonce 还没提交的旧签名扣的那次可以给重签用。
    withdrawn(&db, user, 0, "signed", 0).await;
    assert_eq!(stub_nonce(&s, user, exchange, 0).await, Ok(FEE), "re-sign");
    sqlx::query("UPDATE sponsorships SET status = 'included'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        stub_nonce(&s, user, exchange, 1).await,
        Err(ErrorCode::WithdrawCreditRequired)
    );

    // 完成担保交易得到次数；滚动 24 小时内最多 3 次（25 小时前那次不算），有次数也不行。
    finish_orders(&db, user, 4).await;
    withdrawn(&db, user, 1, "included", 25 * 3600).await;
    withdrawn(&db, user, 2, "failed", 60).await;
    assert_eq!(stub_nonce(&s, user, exchange, 3).await, Ok(FEE));
    withdrawn(&db, user, 3, "included", 60).await;
    assert_eq!(
        stub_nonce(&s, user, exchange, 4).await,
        Err(ErrorCode::SponsorQuotaExceeded)
    );
    // 代发前作废的签名在过期对账前仍可能经第三方上链，照占当天次数；对账确认过期没上链后不占。
    sqlx::query("UPDATE sponsorships SET status = 'dropped' WHERE nonce = 3")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        stub_nonce(&s, user, exchange, 4).await,
        Err(ErrorCode::SponsorQuotaExceeded)
    );
    sqlx::query("UPDATE sponsorships SET status = 'expired' WHERE nonce = 3")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(stub_nonce(&s, user, exchange, 4).await, Ok(FEE));
}
