//! 出网规则（docs/runbooks/origin-protection.md「出站」）：源站只连固定的第三方服务，永远不抓取用户提供的网址
//! ——请求会从源站 IP 发出，等于把 IP 告诉对方。以后确实需要时，经独立的出网代理（不在源站上）转发。
//!
//! 本测试扫描所有程序与库的源码，凡是新建出站网络客户端的地方都必须登记在 `ALLOWED` 里（含次数），
//! 新增或挪动都会让测试失败，由代码审查确认目标是固定服务、地址来自运维配置而不是用户输入。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 新建出站连接的写法。
const PATTERNS: &[&str] = &[
    "Client::new(",
    "Client::builder(",
    "ClientBuilder::new(",
    "reqwest::get(",
    "RpcClient::new_http",
    "ProviderBuilder::new(",
    "connect_http(",
    "connect_ws(",
    "WsConnect::new(",
    "TcpStream::connect",
    "TcpSocket::new",
    "UdpSocket::bind",
    "SmtpTransport::",
    "Resend::new(",
    "connect_async",
    "Bot::new(",
    "hyper_util::client",
];

/// `(文件, 写法, 次数, 连到哪里)`
const ALLOWED: &[(&str, &str, usize, &str)] = &[
    (
        "bins/api/src/main.rs",
        "Client::new(",
        1,
        "Turnstile siteverify (Cloudflare)",
    ),
    (
        "bins/api/src/routes/aa.rs",
        "Client::builder(",
        1,
        "local forum-sponsor (loopback)",
    ),
    (
        "bins/indexer/src/ws.rs",
        "ProviderBuilder::new(",
        1,
        "RPC provider WebSocket (CHAIN_RPC_*_WS)",
    ),
    (
        "bins/indexer/src/ws.rs",
        "connect_ws(",
        1,
        "RPC provider WebSocket (CHAIN_RPC_*_WS)",
    ),
    (
        "bins/indexer/src/ws.rs",
        "WsConnect::new(",
        1,
        "RPC provider WebSocket (CHAIN_RPC_*_WS)",
    ),
    (
        "crates/chain/src/rpc.rs",
        "Client::builder(",
        1,
        "RPC providers and fallback bundler (operator config)",
    ),
    (
        "crates/chain/src/rpc.rs",
        "RpcClient::new_http",
        1,
        "RPC providers (CHAIN_RPC_*_HTTP)",
    ),
    (
        "crates/chain/src/rpc.rs",
        "ProviderBuilder::new(",
        1,
        "RPC providers (CHAIN_RPC_*_HTTP)",
    ),
    (
        "crates/chain/src/anvil.rs",
        "ProviderBuilder::new(",
        1,
        "local Anvil (development and tests)",
    ),
    (
        "crates/chain/src/anvil.rs",
        "connect_http(",
        1,
        "local Anvil (development and tests)",
    ),
    (
        "crates/media/src/storage.rs",
        "Client::builder(",
        1,
        "R2 / S3 endpoint (MEDIA_S3_ENDPOINT)",
    ),
    (
        "crates/mail/src/resend.rs",
        "Resend::new(",
        1,
        "Resend HTTP API",
    ),
    (
        "crates/mail/src/postmark.rs",
        "Client::builder(",
        1,
        "Postmark HTTP API",
    ),
    (
        "crates/mail/src/smtp.rs",
        "SmtpTransport::",
        1,
        "local Mailpit only (loopback enforced)",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn outbound_clients_are_only_built_in_reviewed_places() {
    let backend = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for group in ["bins", "crates"] {
        for member in std::fs::read_dir(backend.join(group)).unwrap().flatten() {
            // 只看会进生产程序的 src/；tests/ 里的集成测试不在源站上运行
            rust_files(&member.path().join("src"), &mut files);
        }
    }
    assert!(files.len() > 50, "scanned only {} files", files.len());

    let mut found = BTreeMap::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        let rel = file
            .strip_prefix(&backend)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for pattern in PATTERNS {
            let n = source.matches(pattern).count();
            if n > 0 {
                found.insert((rel.clone(), *pattern), n);
            }
        }
    }
    let allowed: BTreeMap<_, _> = ALLOWED
        .iter()
        .map(|(file, pattern, n, _)| (((*file).to_owned(), *pattern), *n))
        .collect();

    let unexpected: Vec<_> = found
        .iter()
        .filter(|(k, n)| allowed.get(*k) != Some(n))
        .map(|((file, pattern), n)| format!("{file}: {pattern} x{n}"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "new outbound network client(s):\n  {}\n\
         The origin may only connect to fixed services configured by operators, never to user-supplied URLs \
         (docs/runbooks/origin-protection.md). If this is such a service, add it to ALLOWED with where it connects.",
        unexpected.join("\n  ")
    );
    let stale: Vec<_> = allowed
        .keys()
        .filter(|k| !found.contains_key(*k))
        .map(|(file, pattern)| format!("{file}: {pattern}"))
        .collect();
    assert!(
        stale.is_empty(),
        "remove stale ALLOWED entries:\n  {}",
        stale.join("\n  ")
    );
}
