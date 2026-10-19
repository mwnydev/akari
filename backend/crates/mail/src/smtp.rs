//! SMTP 通道，只用于开发（本机 Mailpit）。SMTP 提交服务器会把客户端 IP（即源站 IP）写进 `Received` 头，
//! 所以只接受本机地址；生产用 HTTP API 通道（Resend、Postmark），见 docs/runbooks/origin-protection.md「邮件」。
//! SMTP 没有幂等键，极少数情况下重试可能重复投递。

use std::net::IpAddr;

use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use rand::RngExt;

use crate::{Email, MailError, Mailer};

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    /// `url` 必须指向本机，如 `smtp://127.0.0.1:1025`。EHLO 与 Message-ID 用发件地址的域名，不用本机主机名。
    pub fn from_url(url: &str, from: &str) -> Result<Self, MailError> {
        if !is_loopback(url) {
            return Err(MailError::Config(
                "SMTP_URL must point to localhost (SMTP is for development only)".to_owned(),
            ));
        }
        let from: Mailbox = from
            .parse()
            .map_err(|_| MailError::Address("from".to_owned()))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|e| MailError::Transport(e.to_string()))?
            .hello_name(ClientId::Domain(from.email.domain().to_owned()))
            .build();
        Ok(Self { transport, from })
    }

    fn message(&self, email: &Email) -> Result<Message, MailError> {
        let to: Mailbox = email
            .to
            .parse()
            .map_err(|_| MailError::Address("to".to_owned()))?;
        let id: [u8; 16] = rand::rng().random();
        Message::builder()
            .message_id(Some(format!(
                "<{}@{}>",
                hex::encode(id),
                self.from.email.domain()
            )))
            .from(self.from.clone())
            .to(to)
            .subject(&email.subject)
            .multipart(MultiPart::alternative_plain_html(
                email.text.clone(),
                email.html.clone(),
            ))
            .map_err(|e| MailError::Build(e.to_string()))
    }
}

fn is_loopback(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl Mailer for SmtpMailer {
    fn name(&self) -> &'static str {
        "smtp"
    }

    async fn send(&self, email: &Email) -> Result<(), MailError> {
        let msg = self.message(email)?;
        self.transport
            .send(msg)
            .await
            .map_err(|e| MailError::Transport(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;
    use crate::tests::{email, machine_hostname};

    // 连接池在 tokio 运行时里建立
    #[tokio::test]
    async fn only_localhost_is_accepted() {
        for url in [
            "smtp://127.0.0.1:1025",
            "smtp://localhost:1025",
            "smtp://[::1]:1025",
        ] {
            assert!(
                SmtpMailer::from_url(url, "noreply@akari.test").is_ok(),
                "{url}"
            );
        }
        for url in [
            "smtps://user:pass@smtp.example.com:465",
            "smtp://user:pass@203.0.113.5:587?tls=required",
            "smtp://localhost.example.com:25",
            "not a url",
        ] {
            assert!(
                matches!(
                    SmtpMailer::from_url(url, "noreply@akari.test"),
                    Err(MailError::Config(_))
                ),
                "{url}"
            );
        }
    }

    /// 最小 SMTP 服务器：记下 EHLO 参数与 DATA 内容。
    async fn fake_smtp(listener: TcpListener) -> (String, String) {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        w.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        let mut ehlo = String::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(name) = line.strip_prefix("EHLO ") {
                ehlo = name.to_owned();
            } else if line == "DATA" {
                w.write_all(b"354 go\r\n").await.unwrap();
                let mut data = String::new();
                while let Some(l) = lines.next_line().await.unwrap() {
                    if l == "." {
                        break;
                    }
                    data.push_str(&l);
                    data.push('\n');
                }
                w.write_all(b"250 queued\r\n").await.unwrap();
                return (ehlo, data);
            }
            w.write_all(b"250 ok\r\n").await.unwrap();
        }
        panic!("client hung up before DATA");
    }

    #[tokio::test]
    async fn sent_message_carries_no_origin_identifiers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(fake_smtp(listener));
        let mailer = SmtpMailer::from_url(
            &format!("smtp://127.0.0.1:{port}"),
            "Akari <noreply@akari.test>",
        )
        .unwrap();
        mailer.send(&email()).await.unwrap();
        let (ehlo, data) = server.await.unwrap();

        // 服务商会把 EHLO 名写进 Received 头：必须是站点域名，不能是本机主机名
        assert_eq!(ehlo, "akari.test");
        let (headers, _) = data.split_once("\n\n").unwrap();
        let message_id = headers
            .lines()
            .find_map(|l| l.strip_prefix("Message-ID: "))
            .unwrap();
        let local = message_id
            .strip_prefix('<')
            .and_then(|s| s.strip_suffix("@akari.test>"))
            .unwrap();
        assert!(local.len() == 32 && local.bytes().all(|b| b.is_ascii_hexdigit()));
        let lower = headers.to_ascii_lowercase();
        for name in ["received:", "x-originating-ip:", "x-mailer:", "user-agent:"] {
            assert!(!lower.contains(name), "{name} in {headers}");
        }
        if let Some(host) = machine_hostname() {
            assert!(!data.contains(&host), "hostname {host} in message");
        }
    }
}
