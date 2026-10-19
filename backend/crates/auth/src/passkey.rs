//! 钱包、站长后台与论坛裁决 Passkey（plan 第 5 节、ADR 0012）。钱包：webauthn-rs 封装、只接受 ES256（P-256），并取出链上验证器需要的公钥坐标。
//!
//! RP ID 是钱包中心域名、允许的来源只有钱包子域名（关联来源），论坛主站无法请求这个 Passkey。
//!
//! 论坛裁决 Passkey（ADR 0012、0013）是另一组凭证：RP ID 为论坛主域名、只用于签署裁决，断言的 challenge 由
//! [`bind_challenge`] 换成裁决内容的哈希。两组凭证的 Webauthn 实例、用户句柄与数据库 `purpose` 都分开。

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::code::WeakSecret;
use webauthn_rs::prelude::{
    COSEAlgorithm, COSEKeyType, CreationChallengeResponse, CredentialID, ECDSACurve, Passkey,
    PasskeyAuthentication, PasskeyRegistration, RequestChallengeResponse, Url, Uuid, Webauthn,
    WebauthnBuilder, WebauthnError,
};
use webauthn_rs_proto::ResidentKeyRequirement;

pub use webauthn_rs::prelude as webauthn;

/// 构造钱包 Webauthn 实例。`origins` 第一个必须在 `rp_id` 之下；其余为关联来源（Related Origin Requests，
/// 需同时登记在中心域名的 `/.well-known/webauthn`）。
pub fn wallet_webauthn(
    rp_id: &str,
    rp_name: &str,
    origins: &[Url],
) -> Result<Webauthn, WebauthnError> {
    let (first, rest) = origins.split_first().ok_or(WebauthnError::Configuration)?;
    let mut b = WebauthnBuilder::new(rp_id, first)?.rp_name(rp_name);
    for o in rest {
        b = b.append_allowed_origin(o);
    }
    b.build()
}

/// 只接受一个页面来源的 Webauthn 实例：站长后台（RP ID 为后台中心域名，ADR 0012）与论坛裁决
/// （RP ID 为论坛主域名，生产 `akari.cc`）。
pub fn single_origin_webauthn(
    rp_id: &str,
    rp_name: &str,
    origin: &Url,
) -> Result<Webauthn, WebauthnError> {
    WebauthnBuilder::new(rp_id, origin)?
        .rp_name(rp_name)
        .build()
}

/// WebAuthn 用户句柄：由用户 ID 经 HMAC 派生的 UUID。不含个人信息、不可猜，也不用另存一列；
/// 同一用户同一用途的所有 Passkey 句柄相同。钱包、后台与裁决用不同的派生标签：开发环境 RP ID 都是 `localhost`，
/// 句柄相同时认证器会用新凭证覆盖旧凭证。
#[derive(Clone)]
pub struct UserHandles(Hmac<Sha256>);

impl UserHandles {
    /// 钱包 Passkey 的句柄。
    pub fn new(secret: &[u8]) -> Result<Self, WeakSecret> {
        Self::labeled(secret, b"wallet-webauthn-user")
    }

    /// 站长后台 Passkey 的句柄。
    pub fn admin(secret: &[u8]) -> Result<Self, WeakSecret> {
        Self::labeled(secret, b"admin-webauthn-user")
    }

    /// 论坛裁决 Passkey 的句柄。
    pub fn ruling(secret: &[u8]) -> Result<Self, WeakSecret> {
        Self::labeled(secret, b"ruling-webauthn-user")
    }

    fn labeled(secret: &[u8], label: &[u8]) -> Result<Self, WeakSecret> {
        if secret.len() < 32 {
            return Err(WeakSecret);
        }
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret).map_err(|_| WeakSecret)?;
        mac.update(label);
        Ok(Self(mac))
    }

    pub fn of(&self, user_id: i64) -> Uuid {
        let mut mac = self.0.clone();
        mac.update(&user_id.to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid_v8(bytes)
    }
}

/// RFC 9562 自定义版本（v8）UUID：只设版本与变体位。
fn uuid_v8(mut b: [u8; 16]) -> Uuid {
    b[6] = (b[6] & 0x0f) | 0x80;
    b[8] = (b[8] & 0x3f) | 0x80;
    Uuid::from_bytes(b)
}

/// COSE 算法 ES256 的编号。
pub const ES256: i64 = -7;

/// 开始注册：只向浏览器请求 ES256 凭证（Kernel WebAuthn 验证器只认 P-256），且必须是可发现凭证：
/// 紧急取回页（ADR 0007）不连服务器，只能不带 allowCredentials 让用户选出 Passkey、再从签名恢复公钥。
pub fn start_registration(
    webauthn: &Webauthn,
    handle: Uuid,
    user_name: &str,
    display_name: &str,
    exclude: Vec<CredentialID>,
) -> Result<(CreationChallengeResponse, PasskeyRegistration), WebauthnError> {
    let exclude = (!exclude.is_empty()).then_some(exclude);
    let (mut ccr, state) =
        webauthn.start_passkey_registration(handle, user_name, display_name, exclude)?;
    ccr.public_key
        .pub_key_cred_params
        .retain(|p| p.alg == ES256);
    if let Some(sel) = ccr.public_key.authenticator_selection.as_mut() {
        sel.resident_key = Some(ResidentKeyRequirement::Required);
        sel.require_resident_key = true;
    }
    Ok((ccr, state))
}

#[derive(Debug, thiserror::Error)]
#[error("cannot bind the assertion challenge: {0}")]
pub struct BindError(String);

/// 把一次断言的 challenge 换成调用方给定的字节（裁决内容的哈希）：同时改浏览器拿到的选项与服务端保存的状态，
/// 认证器签名的 clientDataJSON 因而包含这个 challenge，`finish_passkey_authentication` 也只认它。
/// 依赖 webauthn-rs 0.5 状态序列化（`danger-allow-state-serialisation`）里 `ast.challenge` 的字段名，所以
/// webauthn-rs 在 Cargo.toml 里钉死版本（`=0.5.5`），升级时下面的测试会先失败；找不到字段时报错（不会退回随机 challenge）。
pub fn bind_challenge(
    options: &mut RequestChallengeResponse,
    state: &PasskeyAuthentication,
    challenge: &[u8],
) -> Result<PasskeyAuthentication, BindError> {
    let err = |e: &dyn std::fmt::Display| BindError(e.to_string());
    let mut v = serde_json::to_value(state).map_err(|e| err(&e))?;
    let slot =
        challenge_slot(&mut v).ok_or_else(|| BindError("state has no ast.challenge".into()))?;
    options.public_key.challenge = challenge.to_vec().into();
    *slot = serde_json::to_value(&options.public_key.challenge).map_err(|e| err(&e))?;
    serde_json::from_value(v).map_err(|e| err(&e))
}

fn challenge_slot(state: &mut serde_json::Value) -> Option<&mut serde_json::Value> {
    state
        .get_mut("ast")
        .and_then(|a| a.get_mut("challenge"))
        .filter(|c| c.is_string())
}

/// 链上验证器需要的 P-256 公钥。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P256Key {
    pub x: [u8; 32],
    pub y: [u8; 32],
}

impl P256Key {
    /// 取出 ES256 / P-256 公钥；其他算法返回 `None`（注册时已只请求 ES256，这里是兜底）。
    pub fn of(passkey: &Passkey) -> Option<Self> {
        let key = passkey.get_public_key();
        if key.type_ != COSEAlgorithm::ES256 {
            return None;
        }
        match &key.key {
            COSEKeyType::EC_EC2(ec) if ec.curve == ECDSACurve::SECP256R1 => Some(Self {
                x: ec.x.as_ref().try_into().ok()?,
                y: ec.y.as_ref().try_into().ok()?,
            }),
            _ => None,
        }
    }

    /// 规范 CBOR 编码的 COSE_Key：`{1: 2 (EC2), 3: -7 (ES256), -1: 1 (P-256), -2: x, -3: y}`，共 77 字节。
    pub fn cose(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(77);
        out.extend_from_slice(&[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20]);
        out.extend_from_slice(&self.x);
        out.extend_from_slice(&[0x22, 0x58, 0x20]);
        out.extend_from_slice(&self.y);
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn handle_is_stable_and_distinct() {
        let h = UserHandles::new(&[1; 32]).unwrap();
        let a = h.of(1);
        assert_eq!(a, h.of(1));
        assert_ne!(a, h.of(2));
        assert_ne!(a, UserHandles::new(&[2; 32]).unwrap().of(1));
        assert!(UserHandles::new(&[1; 31]).is_err());
        assert_eq!(a.get_version_num(), 8);
        assert_ne!(a, UserHandles::admin(&[1; 32]).unwrap().of(1));
        assert_ne!(a, UserHandles::ruling(&[1; 32]).unwrap().of(1));
        assert_ne!(
            UserHandles::admin(&[1; 32]).unwrap().of(1),
            UserHandles::ruling(&[1; 32]).unwrap().of(1)
        );
    }

    #[test]
    fn cose_layout() {
        let k = P256Key {
            x: [1; 32],
            y: [2; 32],
        };
        let c = k.cose();
        assert_eq!(c.len(), 77);
        assert_eq!(&c[10..42], &[1; 32]);
        assert_eq!(&c[45..], &[2; 32]);
    }

    use webauthn_authenticator_rs::WebauthnAuthenticator;
    use webauthn_authenticator_rs::softpasskey::SoftPasskey;

    #[test]
    fn bound_challenge_is_what_the_authenticator_signs() {
        let origin = Url::parse("https://forum.example").unwrap();
        let w = single_origin_webauthn("forum.example", "x", &origin).unwrap();
        let mut auth = WebauthnAuthenticator::new(SoftPasskey::new(true));
        let (ccr, reg) = w
            .start_passkey_registration(Uuid::nil(), "a", "a", None)
            .unwrap();
        let cred = auth.do_registration(origin.clone(), ccr).unwrap();
        let passkey = w.finish_passkey_registration(&cred, &reg).unwrap();

        let (mut options, state) = w
            .start_passkey_authentication(std::slice::from_ref(&passkey))
            .unwrap();
        let bound = bind_challenge(&mut options, &state, &[7; 32]).unwrap();
        assert_eq!(options.public_key.challenge.as_ref(), &[7; 32]);
        let assertion = auth.do_authentication(origin, options).unwrap();
        // 原来的随机 challenge 状态、绑定到另一份内容的状态都不认这份断言，只有绑定到同一内容的状态认
        assert!(w.finish_passkey_authentication(&assertion, &state).is_err());
        let (mut other, s2) = w.start_passkey_authentication(&[passkey]).unwrap();
        let other = bind_challenge(&mut other, &s2, &[8; 32]).unwrap();
        assert!(w.finish_passkey_authentication(&assertion, &other).is_err());
        assert!(w.finish_passkey_authentication(&assertion, &bound).is_ok());
    }

    /// 绑定靠的是 webauthn-rs 内部状态的布局：`ast.challenge` 就是发给浏览器的 challenge（同一种编码）。
    /// 依赖升级改了这个字段（改名、换编码、挪位置）时这里失败，而 `bind_challenge` 本身对缺字段一律报错。
    #[test]
    fn webauthn_state_layout_is_what_bind_challenge_expects() {
        let origin = Url::parse("https://forum.example").unwrap();
        let w = single_origin_webauthn("forum.example", "x", &origin).unwrap();
        let mut auth = WebauthnAuthenticator::new(SoftPasskey::new(true));
        let (ccr, reg) = w
            .start_passkey_registration(Uuid::nil(), "a", "a", None)
            .unwrap();
        let cred = auth.do_registration(origin, ccr).unwrap();
        let passkey = w.finish_passkey_registration(&cred, &reg).unwrap();
        let (options, state) = w.start_passkey_authentication(&[passkey]).unwrap();
        let mut v = serde_json::to_value(&state).unwrap();
        assert_eq!(
            challenge_slot(&mut v).cloned(),
            Some(serde_json::to_value(&options.public_key.challenge).unwrap()),
            "webauthn-rs state layout changed: {v}"
        );

        // 缺字段或类型不对：不绑定
        for broken in [
            serde_json::json!({}),
            serde_json::json!({"ast": {}}),
            serde_json::json!({"ast": {"challenge": [1, 2]}}),
            serde_json::json!({"ast": {"chal": "AAAA"}}),
        ] {
            let mut b = broken.clone();
            assert!(challenge_slot(&mut b).is_none(), "{broken}");
        }
    }

    #[test]
    fn rp_id_must_cover_first_origin() {
        let wallet = Url::parse("https://wallet.example").unwrap();
        let other = Url::parse("https://wallet.other.example").unwrap();
        assert!(wallet_webauthn("wallet.example", "x", &[wallet.clone(), other]).is_ok());
        assert!(wallet_webauthn("forum.example", "x", &[wallet]).is_err());
        assert!(wallet_webauthn("wallet.example", "x", &[]).is_err());
    }
}
