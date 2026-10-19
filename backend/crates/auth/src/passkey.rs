//! 钱包 Passkey（plan 第 5 节）：webauthn-rs 封装、只接受 ES256（P-256），并取出链上验证器需要的公钥坐标。
//!
//! RP ID 是钱包中心域名、允许的来源只有钱包子域名（关联来源），论坛主站无法请求这个 Passkey。

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::code::WeakSecret;
use webauthn_rs::prelude::{
    COSEAlgorithm, COSEKeyType, CreationChallengeResponse, CredentialID, ECDSACurve, Passkey,
    PasskeyRegistration, Url, Uuid, Webauthn, WebauthnBuilder, WebauthnError,
};

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

/// WebAuthn 用户句柄：由用户 ID 经 HMAC 派生的 UUID。不含个人信息、不可猜，也不用另存一列；
/// 同一用户的所有钱包 Passkey 句柄相同。
#[derive(Clone)]
pub struct UserHandles(Hmac<Sha256>);

impl UserHandles {
    pub fn new(secret: &[u8]) -> Result<Self, WeakSecret> {
        if secret.len() < 32 {
            return Err(WeakSecret);
        }
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret).map_err(|_| WeakSecret)?;
        mac.update(b"wallet-webauthn-user");
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

/// 开始注册：只向浏览器请求 ES256 凭证（Kernel WebAuthn 验证器只认 P-256）。
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
    Ok((ccr, state))
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

    #[test]
    fn rp_id_must_cover_first_origin() {
        let wallet = Url::parse("https://wallet.example").unwrap();
        let other = Url::parse("https://wallet.other.example").unwrap();
        assert!(wallet_webauthn("wallet.example", "x", &[wallet.clone(), other]).is_ok());
        assert!(wallet_webauthn("forum.example", "x", &[wallet]).is_err());
        assert!(wallet_webauthn("wallet.example", "x", &[]).is_err());
    }
}
