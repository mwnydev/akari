//! Postmark HTTP API 通道（与 Resend 互为主备；两家基础设施独立）。经 HTTPS API 提交，邮件由服务商生成，
//! 不带源站 IP 或主机名。Postmark 没有幂等键，极少数情况下重试可能重复投递。

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Email, MailError, Mailer};

const ENDPOINT: &str = "https://api.postmarkapp.com/email";
const TIMEOUT: Duration = Duration::from_secs(15);

pub struct PostmarkMailer {
    http: reqwest::Client,
    endpoint: String,
    server_token: String,
    from: String,
}

/// <https://postmarkapp.com/developer/api/email-api>；不传自定义邮件头，不开打开与点击追踪。
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Request<'a> {
    from: &'a str,
    to: &'a str,
    subject: &'a str,
    html_body: &'a str,
    text_body: &'a str,
    message_stream: &'static str,
    track_opens: bool,
    track_links: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Response {
    error_code: i64,
}

impl PostmarkMailer {
    /// `server_token`：Postmark 服务器的 API 令牌；`from` 形如 `品牌名 <noreply@example.com>`（须是已验证的发件域名）。
    pub fn new(server_token: &str, from: impl Into<String>) -> Result<Self, MailError> {
        Self::with_endpoint(ENDPOINT, server_token, from)
    }

    fn with_endpoint(
        endpoint: &str,
        server_token: &str,
        from: impl Into<String>,
    ) -> Result<Self, MailError> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| MailError::Config(e.to_string()))?;
        Ok(Self {
            http,
            endpoint: endpoint.to_owned(),
            server_token: server_token.to_owned(),
            from: from.into(),
        })
    }
}

impl Mailer for PostmarkMailer {
    fn name(&self) -> &'static str {
        "postmark"
    }

    async fn send(&self, email: &Email) -> Result<(), MailError> {
        let body = Request {
            from: &self.from,
            to: &email.to,
            subject: &email.subject,
            html_body: &email.html,
            text_body: &email.text,
            message_stream: "outbound",
            track_opens: false,
            track_links: "None",
        };
        let res = self
            .http
            .post(&self.endpoint)
            .header("Accept", "application/json")
            .header("X-Postmark-Server-Token", &self.server_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| MailError::Transport(e.to_string()))?;
        let status = res.status();
        // 出错时只记状态码与 Postmark 错误码：错误说明里可能带收件人地址
        let code = res.json::<Response>().await.ok().map(|r| r.error_code);
        match (status.is_success(), code) {
            (true, Some(0)) => Ok(()),
            (_, code) => Err(MailError::Transport(format!(
                "postmark: HTTP {status}, error code {}",
                code.map_or_else(|| "unknown".to_owned(), |c| c.to_string())
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;
    use crate::tests::{email, machine_hostname};

    /// 最小 HTTP 服务器：回一次 `status` + `body`，返回收到的请求头（小写）与请求体。
    async fn fake_api(listener: TcpListener, status: &str, body: &str) -> (String, Value) {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
            head.push_str(&line.to_ascii_lowercase());
        }
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut req = vec![0; len];
        reader.read_exact(&mut req).await.unwrap();
        let reply = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
        (head, serde_json::from_slice(&req).unwrap())
    }

    async fn send_to_fake(
        status: &'static str,
        body: &'static str,
    ) -> (Result<(), MailError>, String, Value) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/email", listener.local_addr().unwrap());
        let server = tokio::spawn(fake_api(listener, status, body));
        let mailer =
            PostmarkMailer::with_endpoint(&endpoint, "server-token", "Akari <noreply@akari.test>")
                .unwrap();
        let result = mailer.send(&email()).await;
        let (head, req) = server.await.unwrap();
        (result, head, req)
    }

    #[tokio::test]
    async fn request_has_only_the_message_and_no_origin_identifiers() {
        let (result, head, req) = send_to_fake(
            "200 OK",
            r#"{"ErrorCode":0,"Message":"OK","MessageID":"x"}"#,
        )
        .await;
        result.unwrap();

        assert!(head.starts_with("post /email "));
        assert!(head.contains("x-postmark-server-token: server-token\r\n"));
        for name in [
            "x-forwarded-for:",
            "x-originating-ip:",
            "via:",
            "forwarded:",
        ] {
            assert!(!head.contains(name), "{name} in {head}");
        }
        if let Some(host) = machine_hostname() {
            assert!(
                !head.contains(&host.to_ascii_lowercase()),
                "hostname in {head}"
            );
        }
        let mut keys: Vec<_> = req.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        // 没有 Headers 字段：不传任何自定义邮件头
        assert_eq!(
            keys,
            [
                "From",
                "HtmlBody",
                "MessageStream",
                "Subject",
                "TextBody",
                "To",
                "TrackLinks",
                "TrackOpens"
            ]
        );
        assert_eq!(req["From"], "Akari <noreply@akari.test>");
        assert_eq!(req["To"], "a@example.com");
        assert_eq!(req["TrackOpens"], false);
        assert_eq!(req["TrackLinks"], "None");
    }

    #[tokio::test]
    async fn api_error_is_reported_without_recipient() {
        let (result, _, _) = send_to_fake(
            "422 Unprocessable Entity",
            r#"{"ErrorCode":300,"Message":"Invalid 'To' address: 'a@example.com'."}"#,
        )
        .await;
        let MailError::Transport(msg) = result.unwrap_err() else {
            panic!("expected transport error");
        };
        assert!(msg.contains("422") && msg.contains("300"), "{msg}");
        assert!(!msg.contains("a@example.com"), "{msg}");
    }
}
