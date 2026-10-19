//! 提回交易所的代付规则（plan 第 5 节「提币地址冷却」）：USDT `transfer` 只代付转到本人已生效的提币地址，
//! 冷却中、已撤销的地址与别人的地址都拒绝；通过的照样收代付费。
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
use forum_sponsor::jsonrpc::Error;
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

fn transfer(to: Address) -> UserOperation {
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
        call_data: kernel::encode(&[call]),
        ..UserOperation::default()
    }
}

async fn stub(s: &Sponsor, user_id: i64, to: Address) -> Result<u128, ErrorCode> {
    match s
        .stub(user_id, &transfer(to), ENTRY_POINT, U64::from(CHAIN_ID))
        .await
    {
        Ok(f) => Ok(SponsorTerms::decode(&f.paymaster_data).unwrap().fee),
        Err(Error::Reject(r)) => Err(r.code),
        Err(Error::Internal(e)) => panic!("internal error: {e}"),
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
