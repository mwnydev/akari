//! 对象存储（S3 兼容服务）：需要 `MEDIA_S3_*` 环境变量（本地见 deploy/dev/compose.yaml 的 `s3` 服务），否则跳过。

#![allow(clippy::unwrap_used, clippy::expect_used)] // 测试代码

use forum_media::{Storage, StorageConfig};

async fn storage() -> Option<Storage> {
    let cfg = forum_testkit::require_s3(StorageConfig::from_env().unwrap())?;
    let s = Storage::new(&cfg).unwrap();
    s.ensure_bucket().await.unwrap();
    Some(s)
}

#[tokio::test]
async fn presigned_put_enforces_type_and_length() {
    let Some(s) = storage().await else { return };
    let http = reqwest::Client::new();
    let key = forum_media::tmp_key(7);
    let body = vec![7u8; 1000];
    let signed = s.presign_upload(&key, "image/png", body.len() as u64);

    // 字节数与声明不符：签名不匹配
    let mut bad = http.put(&signed.url).body(vec![7u8; 999]);
    for (k, v) in &signed.headers {
        if *k != "content-length" {
            bad = bad.header(*k, v);
        }
    }
    assert!(!bad.send().await.unwrap().status().is_success());
    // 类型与声明不符
    let r = http
        .put(&signed.url)
        .header("content-type", "text/html")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert!(!r.status().is_success());
    assert_eq!(s.get(&key, 10_000).await.unwrap(), None);

    let mut ok = http.put(&signed.url).body(body.clone());
    for (k, v) in &signed.headers {
        if *k != "content-length" {
            ok = ok.header(*k, v);
        }
    }
    let r = ok.send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(s.get(&key, 10_000).await.unwrap(), Some(body));
    assert!(s.get(&key, 999).await.is_err(), "size cap while streaming");
    s.delete(&key).await.unwrap();
    assert_eq!(s.get(&key, 10_000).await.unwrap(), None);
}

#[tokio::test]
async fn public_objects_are_immutable_and_readable() {
    let Some(s) = storage().await else { return };
    let webp = format!("not really webp {}", forum_media::tmp_key(1)).into_bytes();
    let key = forum_media::image_key(&webp);
    s.put_public(&key, "image/webp", webp.clone())
        .await
        .unwrap();
    let r = reqwest::get(s.public_url(&key)).await.unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    assert_eq!(
        r.headers().get("cache-control").unwrap(),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(r.bytes().await.unwrap().as_ref(), webp.as_slice());
}
