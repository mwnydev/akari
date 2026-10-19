//! 邮箱验证码登录（plan 第 5 节）、人机验证与钱包 Passkey。

pub mod captcha;
pub mod challenge;
pub mod code;
pub mod passkey;

pub use captcha::{AnyCaptcha, Captcha, NoopCaptcha, Turnstile};
pub use code::{CodeError, CodeStore, WeakSecret};
