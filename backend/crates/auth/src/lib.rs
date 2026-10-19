//! 邮箱验证码登录（plan 第 5 节）与人机验证。

pub mod captcha;
pub mod code;

pub use captcha::{AnyCaptcha, Captcha, NoopCaptcha, Turnstile};
pub use code::{CodeError, CodeStore, WeakSecret};
